use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;

use super::*;

/// The dialect of a shell line, as a pane that runs it through `/bin/sh` reads it.
fn of_shell(shell: &str) -> ShellDialect {
    shell_program(shell).map_or(ShellDialect::Unknown, |program| ShellDialect::of_program(&program))
}

/// Arguments each dialect must hand on whole: spaces, both quotes and PowerShell's typographic ones, backslashes
/// before a quote and at the end, an empty one, every shell's operators and what each would expand unquoted.
const NASTY: [&str; 29] = [
    r"a\\b",
    r"tail\",
    "it'\u{301}s; echo INJECTED",
    "x'\u{fe0f}",
    "y'\u{1f3fb}",
    "\u{600}'",
    "'\u{200d}",
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
            "env -i -u TMUX --chdir /tmp -C /tmp -- HOME=/home/u fish",
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
        ("~/bin/fish -l", ShellDialect::Fish),
        ("/usr/bin/env nu", ShellDialect::Unknown),
        ("/usr/bin/env", ShellDialect::Unknown),
        ("env -u", ShellDialect::Unknown),
        ("env --argv0 tcsh fish", ShellDialect::Fish),
        ("env -a bash pwsh", ShellDialect::PowerShell),
        ("env -abash fish", ShellDialect::Fish),
        ("env --argv0", ShellDialect::Unknown),
        ("env -a", ShellDialect::Unknown),
        ("env -S '#=x bash' fish", ShellDialect::Fish),
        ("env -S '#=x bash'", ShellDialect::Unknown),
        ("env NOTE='use bash' /bin/tcsh", ShellDialect::Posix),
        ("env NOTE=\"use fish\" /bin/bash", ShellDialect::Posix),
        ("env -C '/tmp/use bash' fish", ShellDialect::Fish),
        ("env --chdir \"/tmp/use fish\" tcsh", ShellDialect::Posix),
        ("env --split-string='NOTE=\"use bash\" tcsh -l'", ShellDialect::Posix),
        ("env -S 'NOTE=\"use fish\" bash -l'", ShellDialect::Posix),
        ("env --split-string 'NOTE=\"use bash\" tcsh -l'", ShellDialect::Posix),
        (r#"env NOTE="use \"bash\"" 'fish'"#, ShellDialect::Fish),
        ("env -S", ShellDialect::Unknown),
        ("env --split-string", ShellDialect::Unknown),
        ("env --split-string=", ShellDialect::Unknown),
        ("env -C", ShellDialect::Unknown),
        ("env --chdir", ShellDialect::Unknown),
        ("env NOTE=use\\", ShellDialect::Unknown),
        (r#"env -S "NOTE=\"use bash here\" /bin/tcsh""#, ShellDialect::Posix),
        ("env -- - NOTE='use bash'", ShellDialect::Unknown),
        ("env NOTE='use bash /bin/tcsh", ShellDialect::Unknown),
        ("nu --login", ShellDialect::Unknown),
    ];
    if cfg!(windows) {
        cases.extend([
            ("/opt/my tools/env fish", ShellDialect::Fish),
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
    } else {
        cases.extend([
            ("/opt/my tools/env fish", ShellDialect::Unknown),
            ("'/opt/my tools/env' fish", ShellDialect::Fish),
            ("\"/opt/my tools/fish\" -l", ShellDialect::Fish),
            ("~user/bin/fish", ShellDialect::Unknown),
            ("/bin/fi?h", ShellDialect::Unknown),
        ]);
    }
    for (shell, dialect) in cases {
        assert_eq!(of_shell(shell), dialect, "{shell}");
    }
}

/// Windows runs the value directly, so a path with spaces is read whole and a separator is only part of a word.
#[test]
fn windows_reads_a_shell_value_through_its_spaces() {
    for (shell, program) in [
        (
            "C:/Program Files/PowerShell/7/pwsh.exe -NoLogo",
            Some("C:/Program Files/PowerShell/7/pwsh.exe"),
        ),
        ("pwsh -File C:/scripts/bash", Some("pwsh")),
        ("C:/Program Files/Git/usr/bin/env.exe bash -l", Some("bash")),
        ("/opt/my tools/env fish", Some("fish")),
        ("tcsh;/bin/bash", Some("tcsh;/bin/bash")),
        ("C:/Program Files/nu.exe", None),
        ("", None),
    ] {
        assert_eq!(windows_shell_program(shell).as_deref(), program, "{shell}");
    }
}

/// `/bin/sh` removes quotes before `env` reads its words, `env` takes a long option by a unique prefix, and the
/// program's own arguments may expand.
#[test]
fn env_reads_the_words_that_the_shell_hands_it() {
    for (shell, dialect) in [
        ("/usr/bin/env bash --rcfile ~/.bashrc", ShellDialect::Posix),
        ("env zsh -l 2>/dev/null", ShellDialect::Posix),
        ("env zsh -l 2>&1 >|log", ShellDialect::Posix),
        ("env fish -C \"source $HOME/x\"", ShellDialect::Fish),
        ("env fish -C 'source *.fish' # ; bash", ShellDialect::Fish),
        ("env -S 'NOTE=a*b fish'", ShellDialect::Fish),
        ("env -S 'NOTE={x} fish'", ShellDialect::Fish),
        ("env -S 'fish -C ~/rc'", ShellDialect::Fish),
        ("env -S \"fish # it's\" bash", ShellDialect::Fish),
        ("env -S 'fish\t-l'", ShellDialect::Fish),
        ("env --arg bash fish", ShellDialect::Fish),
        ("env --split='A=1 fish' bash", ShellDialect::Fish),
        ("env --ch /tmp --unse X fish", ShellDialect::Fish),
        (
            "env --ignore-e --default-signal=INT --block-signal fish",
            ShellDialect::Fish,
        ),
        ("env --i bash", ShellDialect::Unknown),
        ("env --=x bash", ShellDialect::Unknown),
        ("env --debug=x bash", ShellDialect::Unknown),
        ("env --help bash", ShellDialect::Unknown),
        ("env --vers bash", ShellDialect::Unknown),
        ("env --bogus bash", ShellDialect::Unknown),
        ("env -x bash", ShellDialect::Unknown),
        ("env A=1 -i bash", ShellDialect::Unknown),
        ("env -'a' bash fish", ShellDialect::Fish),
        ("env -\"a\" bash fish", ShellDialect::Fish),
        ("env --'argv0' bash fish", ShellDialect::Fish),
        (r"env --arg\v0 bash fish", ShellDialect::Fish),
        (r#"env -"u" bash -"C" /tmp fish"#, ShellDialect::Fish),
        ("env -S\"X=1\"' fish -l' bash", ShellDialect::Fish),
        ("env -S 'NOTE=x '\"fish\" bash", ShellDialect::Fish),
        ("env NOTE=1\u{a0}bash fish", ShellDialect::Fish),
        ("env\u{a0}bash", ShellDialect::Unknown),
        (r#"env NOTE="a\b" fish"#, ShellDialect::Fish),
        ("env NOTE=\"use bash", ShellDialect::Unknown),
        (r#"env NOTE="use\"#, ShellDialect::Unknown),
        ("env -S \"'fish\"", ShellDialect::Unknown),
    ] {
        assert_eq!(of_shell(shell), dialect, "{shell}");
    }
}

#[test]
fn unsupported_env_expansions_leave_the_receiver_unknown() {
    for shell in [
        r"env $'/bin/tcsh\x00=x' bash",
        "env {NOTE=x,/bin/tcsh} bash",
        "env {NOTE=x,}/bin/tcsh bash",
        "env $RECEIVER bash",
        "env ${RECEIVER} bash",
        "env $(printf /bin/tcsh) bash",
        "env `printf /bin/tcsh` bash",
        "env /bin/[t]csh bash",
        "env /bin/*csh bash",
        "env /bin/?csh bash",
        "env ~/tcsh bash",
        "env <(printf /bin/tcsh) bash",
        "env \"$RECEIVER\" bash",
        "env \"`printf /bin/tcsh`\" bash",
        "env -u 2>/dev/null bash /bin/tcsh",
        "env -C/;tcsh; bash",
        "env -u x|tcsh bash",
        "env -u x&tcsh bash",
        "env X=1) bash",
        "env X=a] bash",
        "env X=a} bash",
        "env bash -c :;/bin/tcsh",
        "env bash | tcsh",
        "env bash >&2|tcsh",
        "env bash & tcsh",
        "env bash (tcsh)",
        "env bash $(tcsh)",
        "env bash \"$(tcsh)\"",
        "env bash `tcsh`",
        "env bash ${X:- #}; tcsh",
        "env bash \"${X}\"",
        "env bash $'\\'' ; tcsh ; $'\\''",
        "env bash\ntcsh",
        "env bash \\\ntcsh",
        "env bash \"\\\ntcsh\"",
        "env -S '${SHELL}' bash",
        r"env -S 'bash\_-l'",
        "env bash -c : # x\n/bin/tcsh",
        "env -S bash # x\n/bin/tcsh",
        "env # x\nbash",
    ] {
        assert_eq!(shell_program(shell), None, "{shell}");
        let dialect = of_shell(shell);
        assert_eq!(dialect, ShellDialect::Unknown, "{shell}");
        assert_eq!(dialect.typed(&["grep", "!needle"]), None, "{shell}");
    }
    for shell in [
        "env NOTE='{literal}' bash",
        "env NOTE=\"{literal}\" bash",
        "env NOTE='$literal' bash",
        "env NOTE='`literal`' bash",
        "env NOTE='*?[literal]~' bash",
        r"env NOTE=\{literal\} bash",
        "env -S '\"#=literal\" bash'",
        "env -S 'NOTE=x#=literal bash'",
        "env bash \n",
        "env bash # $(tcsh); tcsh",
        "env bash # x\n \n",
        "bash -l # x\n",
    ] {
        assert_eq!(shell_program(shell).as_deref(), Some("bash"), "{shell}");
    }
}

/// Outside Windows, `/bin/sh` runs the configured shell, so a second command after it could be the shell that reads
/// the typed line.
#[test]
fn a_shell_followed_by_another_command_is_unknown_outside_windows() {
    for shell in [
        "bash -c : ; /bin/tcsh",
        "bash -c :;/bin/tcsh",
        "bash -c :\n/bin/tcsh",
        "zsh --version; tcsh",
        "bash -c : # x\n/bin/tcsh",
        "fish | tcsh",
        "bash $(tcsh)",
        "tcsh;/bin/bash",
        "/bin/tcsh; /bin/bash",
        "exec fish || /bin/bash",
        "tcsh;/usr/bin/env bash",
        "fish&/bin/bash",
    ] {
        let program = shell_program(shell);
        if cfg!(windows) {
            assert!(program.is_some(), "{shell}");
        } else {
            assert_eq!(program, None, "{shell}");
            assert_eq!(of_shell(shell).typed(&["grep", "!needle"]), None, "{shell}");
        }
    }
    for shell in ["bash -l 2>/dev/null", "fish -C 'source x; y' # z", "zsh -l\n"] {
        assert_ne!(of_shell(shell), ShellDialect::Unknown, "{shell}");
    }
}

/// `/bin/sh` runs the word after the variable assignments that come before it, and hands them to that program.
#[cfg(not(windows))]
#[test]
fn the_shell_is_the_word_after_its_assignments() {
    use crate::command_restore::should_restore_in_program;

    for (shell, program) in [
        ("X=/bin/bash tcsh", "tcsh"),
        ("FOO=/bin/bash fish", "fish"),
        ("X=/usr/bin/fish bash -l", "bash"),
        ("A=1 _b2='x y' ~/bin/zsh", "/bin/zsh"),
        ("X=1 env fish", "fish"),
    ] {
        assert_eq!(shell_program(shell).as_deref(), Some(program), "{shell}");
    }
    assert_eq!(of_shell("FOO=/bin/bash fish"), ShellDialect::Fish);
    let history = ["grep".to_owned(), "!needle".to_owned()];
    assert!(!should_restore_in_program(
        &history,
        &[] as &[&str],
        shell_program("X=/bin/bash tcsh").as_deref()
    ));
    for shell in ["X=1", "X=$(tcsh) bash", "X=1;tcsh bash", "X=1 Y=2"] {
        assert_eq!(shell_program(shell), None, "{shell}");
    }
}

/// A shell run directly, not through `/bin/sh`, is its executable's path whole.
#[test]
fn a_shell_path_run_directly_keeps_its_spaces_and_brackets() {
    for (path, dialect) in [
        ("/opt/my shells/zsh", ShellDialect::Posix),
        ("/opt/fish tools/zsh", ShellDialect::Posix),
        ("/opt/tools (x64)/fish", ShellDialect::Fish),
    ] {
        assert_eq!(ShellDialect::of_program(path), dialect, "{path}");
    }
}

#[test]
fn bash_zsh_and_fish_read_a_bracketed_paste_outside_windows() {
    for name in ["bash", "/bin/bash", "-zsh", "/usr/bin/fish", "rbash"] {
        assert_eq!(ShellDialect::reads_bracketed_paste(name), !cfg!(windows), "{name}");
    }
    for name in ["sh", "/bin/dash", "pwsh", "cmd.exe", "nu", "bashful"] {
        assert!(!ShellDialect::reads_bracketed_paste(name), "{name}");
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
            .is_some_and(|key| key.starts_with("NGTERM_") || key.starts_with("AGTERM_"))
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
