#![cfg(feature = "local")]
//! A rerun restore's replay typed into a real shell on a real PTY, as a restored pane types it
//! ([`ngterm_core::command_restore::PaneInput::Replay`], quoted by `ngterm_core::shell_dialect` for the shell the
//! launch runs): the program it starts receives exactly the captured argv. On Windows the shells a pane runs there:
//! PowerShell 7 (`pwsh.exe`, when on `PATH`; every CI runner carries it), Windows PowerShell 5.1 and `cmd.exe`; on Unix
//! `/bin/sh`. The program is this test binary, which records its arguments ([`argv_recorder`]).

#[expect(dead_code, reason = "the file helpers serve the other test files")]
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ngterm_core::command_restore::PaneInput;
use ngterm_core::ghostty_config::GhosttyConfig;
use ngterm_term::{Host, Launch, Pane, PaneConfig};
use support::{SIZE, TestResult};

/// How long the replay may take: Windows PowerShell can take seconds to start on a CI runner.
const PATIENCE: Duration = Duration::from_secs(60);

/// Where [`argv_recorder`] writes what it was run with.
const RECORD: &str = "REPLAY_PANE_TEST_RECORD";

/// Arguments every shell must hand on whole, typed at its prompt: spaces, both quotes and PowerShell's typographic
/// ones, backslashes before a quote and at the end, an empty one, every shell's operators and what each would expand
/// unquoted. No tab or other control character: a capture holding one is never replayed.
const NASTY: [&str; 20] = [
    "a b",
    "it's \"quoted\"",
    r#"back\"slash"#,
    r"C:\dir with space\",
    "",
    "$HOME",
    "%PATH%",
    "100%",
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

/// The shells a pane of this OS runs, by full path.
fn shells() -> Vec<PathBuf> {
    if cfg!(unix) {
        return vec![PathBuf::from("/bin/sh")];
    }
    let system = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let pwsh = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("pwsh.exe"))
            .find(|file| file.metadata().is_ok_and(|meta| meta.is_file() && meta.len() > 0))
    });
    let mut shells: Vec<PathBuf> = pwsh.into_iter().collect();
    shells.push(system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    shells.push(system.join(r"System32\cmd.exe"));
    shells
}

/// The argv the replay runs: this test binary as [`argv_recorder`], then [`NASTY`].
fn replayed() -> TestResult<Vec<String>> {
    let mut argv = vec![
        std::env::current_exe()?.display().to_string(),
        "--ignored".to_owned(),
        "--exact".to_owned(),
        "argv_recorder".to_owned(),
        "--".to_owned(),
    ];
    argv.extend(NASTY.iter().map(|word| (*word).to_owned()));
    Ok(argv)
}

/// `shell` launched as a pane's login shell with `argv` replayed into it, its record going to `record`, `env` added
/// to its environment.
fn launch(shell: &Path, argv: &[String], record: &Path, dir: &Path, env: &[(String, String)]) -> TestResult<Pane> {
    let isolation = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("NGTERM_") || name.starts_with("AGTERM_"))
        .collect();
    let launch = Launch {
        initial_input: Some(PaneInput::Replay(argv.to_vec())),
        working_directory: Some(dir.to_path_buf()),
        env: [(RECORD.to_owned(), record.display().to_string())]
            .into_iter()
            .chain(env.iter().cloned())
            .collect(),
        env_remove: isolation,
        ..Launch::default()
    };
    let host = Host::new(&GhosttyConfig::default(), Some(shell.display().to_string()), None, None);
    Ok(Pane::launch(&launch, &host, &PaneConfig::new(SIZE))?)
}

/// The arguments recorded in `record`, waited for; the pane's screen in the failure.
fn wait_record(pane: &Pane, record: &Path) -> Vec<String> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(bytes) = std::fs::read(record) {
            let text = String::from_utf8_lossy(&bytes);
            return text.split('\0').map(str::to_owned).collect();
        }
        assert!(
            Instant::now() < deadline,
            "nothing recorded; the screen:\n{}",
            pane.frame().frame.lines().join("\n")
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_replay_typed_into_each_shell_runs_the_exact_argv() -> TestResult {
    let dir = tempfile::tempdir()?;
    let argv = replayed()?;
    for (index, shell) in shells().iter().enumerate() {
        let record = dir.path().join(format!("record-{index}"));
        let pane = launch(shell, &argv, &record, dir.path(), &[])?;
        let recorded = wait_record(&pane, &record);
        assert_eq!(recorded, NASTY, "{}", shell.display());
    }
    Ok(())
}

/// The line-editing shells this host has, by full path; macOS' `/bin/bash` 3.2 has no bracketed paste, so only a
/// newer bash counts there.
fn line_editors() -> Vec<PathBuf> {
    let bash: &[&str] = if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin/bash", "/usr/local/bin/bash"]
    } else {
        &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
    };
    let zsh: &[&str] = &["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"];
    let fish: &[&str] = &[
        "/usr/bin/fish",
        "/bin/fish",
        "/usr/local/bin/fish",
        "/opt/homebrew/bin/fish",
    ];
    [("fish", fish), ("zsh", zsh), ("bash", bash)]
        .into_iter()
        .filter_map(|(name, paths)| {
            let found = paths.iter().map(PathBuf::from).find(|path| path.exists());
            if found.is_none() {
                eprintln!("SKIPPED: no {name} on this host");
            }
            found
        })
        .collect()
}

/// The environment that makes `shell` read a throwaway config from `dir`, in which the `'` key is bound as an
/// autopair plugin binds it: it types a pair of quotes and puts the cursor between them.
fn quote_pairing(shell: &Path, dir: &Path) -> TestResult<Vec<(String, String)>> {
    let home = dir.join("home");
    std::fs::create_dir_all(&home)?;
    let mut env = vec![("HOME".to_owned(), home.display().to_string())];
    match shell.file_name().and_then(|name| name.to_str()) {
        Some("fish") => {
            let conf = dir.join("config/fish/conf.d");
            std::fs::create_dir_all(&conf)?;
            // the name of a vendor autopair plugin too, which this file overrides
            std::fs::write(
                conf.join("autopair.fish"),
                "function _pair_quote\n    commandline --insert \"''\"\n    \
                 commandline --cursor (math (commandline --cursor) - 1)\nend\nbind \"'\" _pair_quote\n",
            )?;
            env.push(("XDG_CONFIG_HOME".to_owned(), dir.join("config").display().to_string()));
        }
        Some("zsh") => {
            let zdotdir = dir.join("zdotdir");
            std::fs::create_dir_all(&zdotdir)?;
            std::fs::write(zdotdir.join(".zshenv"), "unsetopt GLOBAL_RCS\n")?;
            std::fs::write(
                zdotdir.join(".zshrc"),
                "pair-quote() { LBUFFER+=\"'\"; RBUFFER=\"'$RBUFFER\"; }\n\
                 zle -N pair-quote\nbindkey \"'\" pair-quote\n",
            )?;
            env.push(("ZDOTDIR".to_owned(), zdotdir.display().to_string()));
        }
        _ => {
            let inputrc = dir.join("inputrc");
            std::fs::write(&inputrc, "\"'\": \"\\C-v'\\C-v'\\C-b\"\n")?;
            env.push(("INPUTRC".to_owned(), inputrc.display().to_string()));
        }
    }
    Ok(env)
}

/// A key-bound plugin (fish's autopair, zsh-autopair, a readline macro) rewrites the quotes a replay would type, so a
/// shell that reads bracketed paste gets the replay pasted: the plugin's keys never run on it.
#[test]
fn a_replay_runs_the_exact_argv_under_a_plugin_bound_to_the_quote_key() -> TestResult {
    let dir = tempfile::tempdir()?;
    let argv = replayed()?;
    for (index, shell) in line_editors().iter().enumerate() {
        let home = dir.path().join(format!("shell-{index}"));
        let env = quote_pairing(shell, &home)?;
        let record = dir.path().join(format!("record-{index}"));
        let pane = launch(shell, &argv, &record, dir.path(), &env)?;
        let recorded = wait_record(&pane, &record);
        assert_eq!(recorded, NASTY, "{}", shell.display());
    }
    Ok(())
}

/// Not a test of its own: the program the replay runs, which records its arguments after `--`, joined with NULs, in
/// the file [`RECORD`] names (written whole, then renamed into place).
#[test]
#[ignore = "the replay test's child"]
fn argv_recorder() {
    let Some(record) = std::env::var_os(RECORD) else {
        return;
    };
    let argv: Vec<String> = std::env::args().skip_while(|arg| arg != "--").skip(1).collect();
    let record = PathBuf::from(record);
    let partial = record.with_extension("partial");
    std::fs::write(&partial, argv.join("\0")).unwrap();
    std::fs::rename(&partial, &record).unwrap();
}
