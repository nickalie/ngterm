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

/// `shell` launched as a pane's login shell with `argv` replayed into it, its record going to `record`.
fn launch(shell: &Path, argv: &[String], record: &Path, dir: &Path) -> TestResult<Pane> {
    let isolation = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("NGTERM_") || name.starts_with("AGTERM_") || name.starts_with("ZMX_"))
        .collect();
    let launch = Launch {
        initial_input: Some(PaneInput::Replay(argv.to_vec())),
        working_directory: Some(dir.to_path_buf()),
        env: vec![(RECORD.to_owned(), record.display().to_string())],
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
            let text = String::from_utf8(bytes).unwrap();
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
        let pane = launch(shell, &argv, &record, dir.path())?;
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
