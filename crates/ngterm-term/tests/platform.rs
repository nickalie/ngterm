//! A pane on a real PTY on every OS (openpty on Unix, the pseudoconsole on Windows), launched as a pane's command
//! line is (`/bin/sh -c`, or `cmd.exe /D /S /C "<line>"`): its output on screen, an echo round trip, a resize reaching
//! the child, exit codes, selection and search over what the child printed, and the password-input detection.
//!
//! The other test files drive the child with POSIX shell scripts and check the VT byte for byte, so they run on Unix
//! only: Windows' pseudoconsole renders the child's output itself and re-emits it, which they cannot pin.

#[expect(dead_code, reason = "the file helpers serve the other test files")]
mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_core::ghostty_config::GhosttyConfig;
use ngterm_term::{ExitStatus, Frame, GridPoint, GridSize, Host, Launch, Pane, PaneConfig, PaneEvent, SelectionText};
use support::{SIZE, TestResult};

/// How long a wait may take: Windows PowerShell can take seconds to start on a CI runner.
const PATIENCE: Duration = Duration::from_secs(60);

/// The command line for this OS: `unix` for `/bin/sh -c`, `windows` for `cmd.exe`.
fn line(unix: &str, windows: &str) -> String {
    if cfg!(windows) { windows } else { unix }.to_owned()
}

/// `command` launched as a pane's command line, isolated from any ngterm the tests run under.
fn launch(command: &str) -> TestResult<Pane> {
    launch_recording(command, None)
}

/// [`launch`], the program's exit status written to `status_file`.
fn launch_recording(command: &str, status_file: Option<PathBuf>) -> TestResult<Pane> {
    let isolation = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("NGTERM_") || name.starts_with("AGTERM_") || name.starts_with("ZMX_"))
        .collect();
    let launch = Launch {
        command: Some(command.to_owned()),
        working_directory: Some(std::env::temp_dir()),
        env_remove: isolation,
        status_file,
        ..Launch::default()
    };
    let host = Host::new(&GhosttyConfig::default(), None, None, None);
    Ok(Pane::launch(&launch, &host, &PaneConfig::new(SIZE))?)
}

/// Poll the pane's frame until `done` holds; the last frame.
fn wait_frame(pane: &Pane, mut done: impl FnMut(&Frame) -> bool) -> Arc<Frame> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let frame = pane.frame().frame;
        if done(&frame) {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; the screen:\n{}",
            frame.lines().join("\n")
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The row and column where `needle` shows on screen, waited for.
fn wait_text(pane: &Pane, needle: &str) -> (u32, u16) {
    let frame = wait_frame(pane, |frame| frame.lines().iter().any(|line| line.contains(needle)));
    let lines = frame.lines();
    let (row, text) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .unwrap_or_else(|| unreachable!("waited for it"));
    let column = text[..text.find(needle).unwrap_or(0)].chars().count();
    (
        u32::try_from(row).unwrap_or(u32::MAX),
        u16::try_from(column).unwrap_or(u16::MAX),
    )
}

/// Collect the pane's events until `done` holds for them; all of them.
fn wait_events(pane: &Pane, mut done: impl FnMut(&[PaneEvent]) -> bool) -> Vec<PaneEvent> {
    let deadline = Instant::now() + PATIENCE;
    let mut events = Vec::new();
    loop {
        events.extend(pane.take_events());
        if done(&events) {
            return events;
        }
        assert!(Instant::now() < deadline, "timed out; events so far: {events:#?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The pane's exit, waited for.
fn exit_of(pane: &Pane) -> ExitStatus {
    let events = wait_events(pane, |events| {
        events.iter().any(|event| matches!(event, PaneEvent::Exited(_)))
    });
    events
        .into_iter()
        .find_map(|event| match event {
            PaneEvent::Exited(status) => Some(status),
            _ => None,
        })
        .unwrap_or_else(|| unreachable!("waited for it"))
}

/// A command that prints `text` and then keeps its pane for 30 s.
fn print_and_stay(text: &str) -> String {
    line(
        &format!("printf '%s' '{text}'; exec sleep 30"),
        &format!("echo {text}& ping -n 30 127.0.0.1 >NUL"),
    )
}

#[test]
fn a_command_line_runs_with_its_quotes_as_written() -> TestResult {
    // cmd.exe's echo prints the quotes it was given: only a command text passed as written keeps them
    let pane = launch(&line(
        "echo '\"quoted words\"'; exec sleep 30",
        "echo \"quoted words\"& ping -n 30 127.0.0.1 >NUL",
    ))?;
    wait_text(&pane, "\"quoted words\"");
    Ok(())
}

#[test]
fn typed_input_round_trips_through_the_child() -> TestResult {
    // the typed line reaches the child, which prints it back tagged (`call` expands the variable after `set /p`)
    let pane = launch(&line(
        "read line; echo \"got:$line\"; exec sleep 30",
        "set /p line=& call echo got:%line%& ping -n 30 127.0.0.1 >NUL",
    ))?;
    pane.write(b"ping\r")?;
    wait_text(&pane, "got:ping");
    Ok(())
}

#[test]
fn a_resize_reaches_the_child() -> TestResult {
    let pane = launch(&line(
        "read _; echo \"SIZE=$(stty size | tr ' ' x)\"; exec sleep 30",
        "set /p _=& powershell.exe -NoLogo -NoProfile -NonInteractive -Command \"'SIZE=' + [Console]::WindowHeight + \
         'x' + [Console]::WindowWidth\"& ping -n 30 127.0.0.1 >NUL",
    ))?;
    pane.resize(GridSize {
        cols: 100,
        rows: 30,
        ..SIZE
    })?;
    pane.write(b"\r")?;
    wait_text(&pane, "SIZE=30x100");
    Ok(())
}

#[test]
fn the_exit_code_is_the_programs() -> TestResult {
    assert_eq!(exit_of(&launch("exit 7")?).code, 7);
    assert_eq!(exit_of(&launch("exit 0")?).code, 0);
    // a command that is not found: the shell's own status for it (127; `cmd /C` ends with 1)
    let missing = exit_of(&launch("ngterm-no-such-command-xyz")?);
    assert_eq!(missing.code, if cfg!(windows) { 1 } else { 127 });
    Ok(())
}

#[test]
fn a_program_that_ends_by_itself_records_its_status_and_a_hung_up_one_none() -> TestResult {
    // an overlay program's status file on Windows, where no wrapper writes it (`ngterm_core::overlay_capture`)
    let dir = tempfile::tempdir()?;
    let ended = dir.path().join("ended.code");
    let pane = launch_recording("exit 7", Some(ended.clone()))?;
    assert_eq!(exit_of(&pane).code, 7);
    assert_eq!(
        std::fs::read_to_string(&ended)?,
        "7\n",
        "written before the exit is reported"
    );
    let hung_up = dir.path().join("hung-up.code");
    let pane = launch_recording(&print_and_stay("UP"), Some(hung_up.clone()))?;
    wait_text(&pane, "UP");
    drop(pane);
    // the child is reaped on the pane's own threads: give a stray write the time it would take
    std::thread::sleep(Duration::from_millis(500));
    assert!(!hung_up.exists(), "the pane's hangup ended it");
    Ok(())
}

#[test]
fn what_the_child_printed_selects_and_searches() -> TestResult {
    let pane = launch(&print_and_stay("alpha needle omega"))?;
    let (row, column) = wait_text(&pane, "needle");
    assert!(pane.select_word(GridPoint::viewport(column + 1, row))?);
    assert_eq!(pane.selection_text(SelectionText::Raw).as_deref(), Some("needle"));
    assert!(pane.select_line(GridPoint::viewport(column, row))?);
    assert_eq!(
        pane.selection_text(SelectionText::Copy).as_deref(),
        Some("alpha needle omega")
    );
    assert!(pane.search("needle"));
    wait_events(&pane, |events| {
        events
            .iter()
            .any(|event| matches!(event, PaneEvent::SearchTotal(Some(1))))
    });
    assert_eq!(pane.search_state().total, Some(1));
    Ok(())
}

/// Unix: a canonical read with echo off is a password prompt, and the shell's process group is the terminal's
/// foreground. Windows has no password read: the pseudoconsole gives its parent no handle on the console's input
/// mode; and no foreground process group, whose stand-in for a command line is `cmd.exe`, the pane's own program,
/// which has no job control.
#[test]
fn a_read_without_echo_is_password_input_where_the_pty_can_say() -> TestResult {
    let pane = launch(&line(
        "printf ready; stty -echo; read secret; exec sleep 30",
        "echo ready& set /p secret=& ping -n 30 127.0.0.1 >NUL",
    ))?;
    wait_text(&pane, "ready");
    if cfg!(windows) {
        std::thread::sleep(Duration::from_millis(500));
        assert!(!pane.password_input());
        assert_eq!(pane.foreground_pid(), pane.pid());
    } else {
        let deadline = Instant::now() + PATIENCE;
        while !pane.password_input() {
            assert!(Instant::now() < deadline, "the read without echo was never seen");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(pane.foreground_pid().is_some());
    }
    Ok(())
}

/// A `^C` typed as `session.type` types text ([`Pane::type_text`]) interrupts the pane's program: the line
/// discipline's SIGINT on Unix; on Windows the console's Ctrl+C event, which ends `ping` with `STATUS_CONTROL_C_EXIT`.
#[test]
fn a_typed_ctrl_c_interrupts_the_program() -> TestResult {
    use ngterm_term::PaneCommand;

    let (command, started) = if cfg!(windows) {
        (
            PaneCommand::new("ping.exe").args(["-n", "600", "127.0.0.1"]),
            "127.0.0.1",
        )
    } else {
        (
            PaneCommand::new("/bin/sh").args(["-c", "echo ready; exec sleep 600"]),
            "ready",
        )
    };
    let pane = Pane::spawn(&command, &PaneConfig::new(SIZE))?;
    wait_text(&pane, started);
    assert!(pane.type_text("\x03")?, "the typed ^C sent nothing");
    let status = exit_of(&pane);
    if cfg!(windows) {
        const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
        assert_eq!(status.code, STATUS_CONTROL_C_EXIT, "{status:?}");
    } else {
        assert!(status.signal.is_some(), "{status:?}");
    }
    Ok(())
}

/// Windows' stand-in foreground: an interactive `cmd.exe` at its prompt holds it, a program typed into it holds it
/// while it runs (the oldest one: `ping` here, not anything it starts), and the shell has it back once the program
/// ends.
#[cfg(windows)]
#[test]
fn the_typed_program_stands_in_for_the_foreground_and_the_shell_takes_it_back() -> TestResult {
    use ngterm_host::ProcessTable as _;
    use ngterm_session::process_table::ProcTable;
    use ngterm_term::PaneCommand;

    let argv = |pid: Option<u32>| pid.and_then(|pid| ProcTable::new().argv(i32::try_from(pid).ok()?));
    let wait_for = |pane: &Pane, what: &str, done: &dyn Fn(Option<u32>) -> bool| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let foreground = pane.foreground_pid();
            if done(foreground) {
                return foreground;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: the foreground is {foreground:?}, {:?}",
                argv(foreground)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let command = PaneCommand::new("cmd.exe").args(["/D", "/Q", "/K", "echo ready"]);
    let pane = Pane::spawn(&command, &PaneConfig::new(SIZE))?;
    wait_text(&pane, "ready");
    let shell = pane.pid();
    wait_for(&pane, "the shell at its prompt", &|foreground| foreground == shell);
    // about four seconds, then the prompt again
    pane.write(b"ping -n 5 127.0.0.1\r")?;
    let program = wait_for(&pane, "the typed ping", &|foreground| {
        foreground.is_some() && foreground != shell
    });
    let typed = argv(program).ok_or("the ping's argv")?;
    assert!(typed[0].to_ascii_lowercase().starts_with("ping"), "{typed:?}");
    wait_for(&pane, "the shell after the ping", &|foreground| foreground == shell);
    Ok(())
}
