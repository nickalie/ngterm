use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;

use super::*;

/// Arguments each dialect must hand on whole: spaces, both quotes and PowerShell's typographic ones, backslashes
/// before a quote and at the end, an empty one, every shell's operators and what each would expand unquoted.
const NASTY: [&str; 22] = [
    "a b",
    "it's \"quoted\"",
    r#"back\"slash"#,
    r"C:\dir with space\",
    r"C:\dir\",
    "",
    "$HOME",
    "%PATH%",
    "100%",
    "%",
    "50% off %",
    "pipe|amp&",
    "semi;colon",
    "<in>out",
    "^caret^",
    "(paren)",
    "bang!",
    "`tick` @splat --%",
    "\u{2018}curly\u{2019}",
    "*.glob ?",
    "~tilde #hash",
    "\"",
];

/// Where [`argv_recorder`] writes what it was run with.
const RECORD: &str = "SHELL_DIALECT_TEST_RECORD";

#[test]
fn a_shell_is_known_by_its_programs_name() {
    let cases = [
        ("/bin/sh", ShellDialect::Posix),
        ("-zsh", ShellDialect::Posix),
        ("/usr/local/bin/bash", ShellDialect::Posix),
        ("/bin/tcsh", ShellDialect::Posix),
        ("/usr/bin/ksh93", ShellDialect::Posix),
        ("/usr/local/bin/loksh", ShellDialect::Posix),
        ("rbash", ShellDialect::Posix),
        ("posh", ShellDialect::Posix),
        ("/usr/bin/fish", ShellDialect::Fish),
        ("pwsh", ShellDialect::PowerShell),
        ("/usr/bin/pwsh", ShellDialect::PowerShell),
        ("PowerShell.EXE", ShellDialect::PowerShell),
        ("cmd.exe", ShellDialect::Cmd),
        ("CMD", ShellDialect::Cmd),
        ("sh.exe", ShellDialect::Posix),
        ("nu", ShellDialect::Unknown),
        ("/usr/bin/xonsh", ShellDialect::Unknown),
        ("", ShellDialect::Unknown),
    ];
    for (program, dialect) in cases {
        assert_eq!(ShellDialect::of_program(program), dialect, "{program}");
    }
}

#[test]
fn a_configured_shell_is_read_through_its_spaces_and_arguments() {
    let mut cases = vec![
        ("pwsh -NoLogo", ShellDialect::PowerShell),
        ("/usr/bin/fish -l", ShellDialect::Fish),
        ("/usr/bin/env fish", ShellDialect::Fish),
        ("/usr/bin/env zsh", ShellDialect::Posix),
        ("env -S zsh -l", ShellDialect::Posix),
        ("/usr/bin/env -S 'fish -l'", ShellDialect::Fish),
        ("env -Sbash --login", ShellDialect::Posix),
        ("env --split-string=pwsh -NoLogo", ShellDialect::PowerShell),
        (
            "env -i HOME=/home/u -u TMUX --chdir /tmp -C /tmp -- fish",
            ShellDialect::Fish,
        ),
        ("env -iuTMUX -P /opt/bin ksh93", ShellDialect::Posix),
        ("env --unset=TMUX - LANG=C env loksh", ShellDialect::Posix),
        ("/usr/bin/env -- FOO=bar /bin/zsh", ShellDialect::Posix),
        ("env -i -- - LANG=C TERM=xterm fish -l", ShellDialect::Fish),
        ("env -- FOO=bar", ShellDialect::Unknown),
        ("bash --rcfile /etc/env", ShellDialect::Posix),
        ("/bin/zsh -c /opt/env", ShellDialect::Posix),
        ("fish -C /etc/bash", ShellDialect::Fish),
        ("/opt/my tools/env fish", ShellDialect::Fish),
        ("/usr/bin/env nu", ShellDialect::Unknown),
        ("/usr/bin/env", ShellDialect::Unknown),
        ("env -u", ShellDialect::Unknown),
        ("nu --login", ShellDialect::Unknown),
    ];
    if cfg!(windows) {
        cases.extend([
            (r"C:\Program Files\Git\usr\bin\ENV.EXE bash -l", ShellDialect::Posix),
            (r"C:\Program Files\PowerShell\7\pwsh.exe", ShellDialect::PowerShell),
            (r"C:\Program Files\Git\bin\sh.exe --login", ShellDialect::Posix),
            (r"C:\Windows\System32\cmd.exe", ShellDialect::Cmd),
            (r"pwsh -File C:\scripts\start.ps1", ShellDialect::PowerShell),
            (r"pwsh -File C:\scripts\env", ShellDialect::PowerShell),
            (r"pwsh -File C:\scripts\bash", ShellDialect::PowerShell),
            (
                r"C:\Program Files\PowerShell\7\pwsh.exe -File C:\tools\env",
                ShellDialect::PowerShell,
            ),
        ]);
    }
    for (shell, dialect) in cases {
        assert_eq!(ShellDialect::of_shell(shell), dialect, "{shell}");
    }
}

#[test]
fn each_dialect_submits_with_its_own_enter() {
    assert_eq!(
        ShellDialect::Posix.typed(&["sleep", "600"]).as_deref(),
        Some("'sleep' '600'\n")
    );
    assert_eq!(
        ShellDialect::Fish.typed(&["sleep", "600"]).as_deref(),
        Some("'sleep' '600'\n")
    );
    assert_eq!(
        ShellDialect::PowerShell.typed(&["sleep", "600"]).as_deref(),
        Some("& 'sleep' '600'\r")
    );
    assert_eq!(
        ShellDialect::Cmd.typed(&["sleep", "600"]).as_deref(),
        Some("sleep 600\r")
    );
    assert_eq!(
        ShellDialect::Unknown.typed(&["sleep", "600"]).as_deref(),
        Some("sleep 600\r")
    );
    let none: [&str; 0] = [];
    for dialect in [
        ShellDialect::Posix,
        ShellDialect::Fish,
        ShellDialect::PowerShell,
        ShellDialect::Cmd,
        ShellDialect::Unknown,
    ] {
        assert_eq!(dialect.typed(&none), None, "{dialect:?}");
    }
}

#[test]
fn posix_keeps_upstreams_line_and_fish_escapes_its_backslashes() {
    let argv = ["/bin/tool", "it's", r"C:\dir\", "$HOME"];
    assert_eq!(
        ShellDialect::Posix.line(&argv).as_deref(),
        Some(r"'/bin/tool' 'it'\''s' 'C:\dir\' '$HOME'")
    );
    assert_eq!(
        ShellDialect::Fish.line(&argv).as_deref(),
        Some(r"'/bin/tool' 'it\'s' 'C:\\dir\\' '$HOME'")
    );
}

#[test]
fn the_c_runtime_word_quotes_only_what_its_split_would_change() {
    let cases = [
        ("plain", "plain"),
        (r"C:\dir\", r"C:\dir\"),
        ("", r#""""#),
        ("a b", r#""a b""#),
        ("tab\there", "\"tab\there\""),
        (r#"say "hi""#, r#""say \"hi\"""#),
        (r#"back\"slash"#, r#""back\\\"slash""#),
        (r"C:\dir with space\", r#""C:\dir with space\\""#),
        (r"a\\b c", r#""a\\b c""#),
        ("\"", r#""\"""#),
    ];
    for (value, quoted) in cases {
        assert_eq!(crt_quoted(value), quoted, "{value}");
    }
}

#[test]
fn powershell_calls_its_program_and_passes_quoted_words_under_legacy_only_when_needed() {
    assert_eq!(
        powershell_call(&[r"C:\tools\top.exe", "-d", "1", "$HOME", "it's"]),
        r"& 'C:\tools\top.exe' '-d' '1' '$HOME' 'it''s'"
    );
    assert_eq!(
        powershell_call(&["top", "a b", "", r#"x"y"#]),
        r#"& { $PSNativeCommandArgumentPassing = 'Legacy' ; & 'top' '"a b"' '""' '"x\"y"' }"#
    );
    // the program is a word too, never a bare keyword
    assert_eq!(powershell_call(&["if", "x"]), "& 'if' 'x'");
}

#[test]
fn cmd_escapes_its_metacharacters_quotes_and_percent_signs() {
    let line = |argv: &[&str]| ShellDialect::Cmd.line(argv);
    assert_eq!(line(&["tool", "a b", "x&y"]).as_deref(), Some(r#"tool ^"a b^" x^&y"#));
    assert_eq!(line(&["tool", r#"x"y"#]).as_deref(), Some(r#"tool ^"x\^"y^""#));
    assert_eq!(line(&["tool", ""]).as_deref(), Some(r#"tool ^"^""#));
    // a caret after each percent sign, which the next character keeps
    assert_eq!(line(&["tool", "%PATH%"]).as_deref(), Some("tool %^PATH%"));
    assert_eq!(line(&["tool", "100%", "x"]).as_deref(), Some("tool 100%^ x"));
    assert_eq!(line(&["tool", "%&"]).as_deref(), Some("tool %^&"));
    assert_eq!(line(&["tool", "^(a|b)<c>!"]).as_deref(), Some("tool ^^^(a^|b^)^<c^>^!"));
    // a program with spaces goes in real quotes; one with a percent sign or a quote is refused
    assert_eq!(
        line(&[r"C:\Program Files (x86)\a&b\tool.exe", "x"]).as_deref(),
        Some(r#""C:\Program Files (x86)\a&b\tool.exe" x"#)
    );
    assert_eq!(line(&[r"C:\%TEMP%\tool.exe", "x"]), None);
    assert_eq!(line(&["", "x"]), None);
}

#[test]
fn an_unknown_shell_gets_plain_words_or_nothing() {
    assert_eq!(
        ShellDialect::Unknown.line(&["/usr/bin/top", "-d", "1.5", "--sort=x"]),
        None,
        "`=` is no plain character"
    );
    assert_eq!(
        ShellDialect::Unknown
            .line(&["/usr/bin/top", "-d", "1.5", "+x:y"])
            .as_deref(),
        Some("/usr/bin/top -d 1.5 +x:y")
    );
    for word in ["a b", "", "$x", "it's", "~", "a,b", "@x", "é"] {
        assert_eq!(ShellDialect::Unknown.line(&["tool", word]), None, "{word}");
    }
    assert_eq!(
        ShellDialect::Unknown.line(&[r"C:\tools\top.exe"]).is_some(),
        cfg!(windows),
        "a backslash is plain on Windows only"
    );
}

/// This test binary run as [`argv_recorder`] with `arguments`.
fn recorder_argv(arguments: &[&str]) -> Vec<String> {
    let mut argv = vec![
        std::env::current_exe().unwrap().display().to_string(),
        "--ignored".to_owned(),
        "--exact".to_owned(),
        "shell_dialect::tests::argv_recorder".to_owned(),
        "--".to_owned(),
    ];
    argv.extend(arguments.iter().map(|word| (*word).to_owned()));
    argv
}

/// `program` on `PATH`, if any.
fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/// Runs `command` with the record file named, without the live ngterm's variables, and reads back the arguments the
/// recorder got.
fn recorded(mut command: Command, record: &Path) -> (Output, Option<Vec<String>>) {
    command.env(RECORD, record).stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|key| key.starts_with("NGTERM_") || key.starts_with("AGTERM_") || key.starts_with("ZMX_"))
        {
            command.env_remove(key);
        }
    }
    let output = command.output().unwrap();
    let argv = fs::read(record)
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap());
    (output, argv)
}

/// Runs `line` in `dialect`'s shell at `program` non-interactively, and the arguments the recorder got.
fn run_line(dialect: ShellDialect, program: &Path, line: &str, record: &Path) -> (Output, Option<Vec<String>>) {
    let mut command = Command::new(program);
    match dialect {
        ShellDialect::PowerShell => {
            let script: Vec<u8> = line.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let encoded = base64::engine::general_purpose::STANDARD.encode(script);
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]);
        }
        ShellDialect::Cmd => cmd_args(&mut command, line),
        _ => {
            command.args(["-c", line]);
        }
    }
    recorded(command, record)
}

/// `cmd.exe /D /S /C "<line>"`, the line as written: `/S` strips the outer quotes and leaves the rest alone.
#[cfg(windows)]
fn cmd_args(command: &mut Command, line: &str) {
    use std::os::windows::process::CommandExt as _;
    command.args(["/D", "/S", "/C"]).raw_arg(format!("\"{line}\""));
}

#[cfg(not(windows))]
fn cmd_args(_command: &mut Command, _line: &str) {
    unreachable!("cmd.exe runs on Windows only")
}

/// The shells of each dialect this host has: POSIX `sh` and fish on Unix, every PowerShell on `PATH` (each CI runner
/// carries `pwsh`; Windows' own `powershell.exe` is 5.1), and `cmd.exe` on Windows.
fn shells() -> Vec<(ShellDialect, PathBuf)> {
    let mut shells = Vec::new();
    if cfg!(unix) {
        shells.push((ShellDialect::Posix, PathBuf::from("/bin/sh")));
        shells.extend(on_path("fish").map(|fish| (ShellDialect::Fish, fish)));
    }
    let names: &[&str] = if cfg!(windows) {
        &["pwsh.exe", "powershell.exe"]
    } else {
        &["pwsh"]
    };
    shells.extend(
        names
            .iter()
            .filter_map(|name| on_path(name))
            .map(|path| (ShellDialect::PowerShell, path)),
    );
    if cfg!(windows) {
        let cmd = std::env::var_os("ComSpec").map_or_else(|| PathBuf::from("cmd.exe"), PathBuf::from);
        shells.push((ShellDialect::Cmd, cmd));
    }
    shells
}

#[test]
fn every_dialects_line_hands_its_shell_the_exact_argv() {
    let dir = tempfile::tempdir().unwrap();
    let argv = recorder_argv(&NASTY);
    for (dialect, shell) in shells() {
        let record = dir.path().join("record.json");
        let _ = fs::remove_file(&record);
        let line = dialect.line(&argv).unwrap();
        let (output, recorded) = run_line(dialect, &shell, &line, &record);
        assert_eq!(
            recorded.as_deref(),
            Some(&argv[5..]),
            "{} ran {line:?}: {output:?}",
            shell.display()
        );
    }
}

/// Not a test of its own: the program the round-trip test runs, which records its arguments after `--` as JSON in the
/// file [`RECORD`] names.
#[test]
#[ignore = "the round-trip test's child"]
fn argv_recorder() {
    let Some(record) = std::env::var_os(RECORD) else {
        return;
    };
    let argv: Vec<String> = std::env::args().skip_while(|arg| arg != "--").skip(1).collect();
    fs::write(record, serde_json::to_string(&argv).unwrap()).unwrap();
}
