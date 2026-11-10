//! `ngtermctl viewer-run WORD`, not a user's command: what a Windows viewer's remote pane or overlay runs locally in
//! place of the `/bin/sh` line a Unix viewer runs ([`ngterm_core::remote::viewer_run`]). It decodes the `WORD` into a
//! [`ViewerRun`] and carries it out on this process's stdio, the pane's console.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::process::{Command, ExitCode, Stdio};

use ngterm_core::remote::viewer_run::{LostLink, SSH_LOST, VIEWER_RUN, ViewerRun, config_probe, with_keep_alive};

/// What `main` does with its arguments.
pub enum Intercepted {
    /// Parse these as the command line.
    Arguments(Vec<OsString>),
    /// Finished with this status.
    Exit(ExitCode),
}

/// `args` (the program name first) unless they ask for a [`VIEWER_RUN`], which is carried out here.
#[must_use]
pub fn intercept(args: Vec<OsString>) -> Intercepted {
    let Some(word) = run_word(&args) else {
        return Intercepted::Arguments(args);
    };
    let run = match word.to_str().ok_or_else(|| format!("{VIEWER_RUN}: not text")) {
        Ok(word) => ViewerRun::decode(word),
        Err(error) => Err(error),
    };
    match run {
        Ok(run) => Intercepted::Exit(exit_code(carry_out(&run, &mut Process))),
        Err(error) => {
            eprintln!("ngtermctl: {error}");
            Intercepted::Exit(ExitCode::from(64))
        }
    }
}

/// The `WORD` of `ngtermctl viewer-run WORD`.
fn run_word(args: &[OsString]) -> Option<&OsStr> {
    match args {
        [_, command, word] if command == VIEWER_RUN => Some(word),
        _ => None,
    }
}

/// `status` as this process's exit code; one no exit code holds is 1.
fn exit_code(status: i32) -> ExitCode {
    ExitCode::from(u8::try_from(status).unwrap_or(1))
}

/// What a run needs of the machine, which the tests stand in for.
trait Machine {
    /// What `argv` printed on stdout, its stdin and stderr closed; `None` when it could not run or failed.
    fn output(&mut self, argv: &[String]) -> Option<String>;
    /// Runs `argv` on this process's stdio and answers with its status.
    fn run(&mut self, argv: &[String]) -> io::Result<i32>;
    /// Prints `text` to this process's stdout.
    fn print(&mut self, text: &str);
    /// Shows `lost`'s bar, then waits until the input ends, unseen.
    fn wait_lost(&mut self, lost: &LostLink);
}

/// Carries out `run` and answers with the status the pane ends with.
fn carry_out(run: &ViewerRun, machine: &mut dyn Machine) -> i32 {
    match run {
        ViewerRun::Program { argv } => status_of(machine.run(argv), argv),
        ViewerRun::Pane { attach, report, lost } => {
            let config = machine.output(&config_probe(attach));
            let argv = with_keep_alive(attach, config.as_deref());
            let status = status_of(machine.run(&argv), &argv);
            match lost {
                Some(lost) if status == SSH_LOST => machine.wait_lost(lost),
                _ => machine.print(&format!("{report} {status}\n")),
            }
            status
        }
    }
}

/// A run's status, or a shell's for a program that could not start: 127 when it is not found, else 126, said on
/// stderr.
fn status_of(ran: io::Result<i32>, argv: &[String]) -> i32 {
    match ran {
        Ok(status) => status,
        Err(error) => {
            let program = argv.first().map_or("", String::as_str);
            eprintln!("ngtermctl: could not run {program}: {error}");
            if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            }
        }
    }
}

/// The real machine: child processes, this process's console.
struct Process;

impl Machine for Process {
    fn output(&mut self, argv: &[String]) -> Option<String> {
        let (program, arguments) = argv.split_first()?;
        let output = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn run(&mut self, argv: &[String]) -> io::Result<i32> {
        let (program, arguments) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "an empty command"))?;
        let status = Command::new(program).args(arguments).status()?;
        Ok(status.code().unwrap_or(1))
    }

    fn print(&mut self, text: &str) {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn wait_lost(&mut self, lost: &LostLink) {
        #[cfg(windows)]
        console::lost_link_modes();
        self.print(&lost.output());
        let _ = io::copy(&mut io::stdin().lock(), &mut io::sink());
    }
}

/// The pane's console modes while a lost link waits: what ssh left them as is not known.
#[cfg(windows)]
mod console {
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, SetConsoleMode,
    };

    /// VT output, so the bar's escapes reach the terminal, and input neither echoed nor gathered into lines, nor a
    /// Ctrl+C that would end the wait: every key is the app's to read as a retry, as under the Unix form's `stty -echo`.
    pub(super) fn lost_link_modes() {
        let output = std::io::stdout();
        let input = std::io::stdin();
        change(output.as_raw_handle(), |mode| {
            mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING
        });
        change(input.as_raw_handle(), |mode| {
            mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT)
        });
    }

    /// `handle`'s console mode through `change`; a handle that is not a console is left alone.
    fn change(handle: std::os::windows::io::RawHandle, change: impl Fn(CONSOLE_MODE) -> CONSOLE_MODE) {
        let mut mode: CONSOLE_MODE = 0;
        // SAFETY: one of this process's standard handles, open for its life; a handle that is not a console fails
        // the call.
        if unsafe { GetConsoleMode(handle, &raw mut mode) } != 0 {
            // SAFETY: as above, with a mode built from its own.
            unsafe { SetConsoleMode(handle, change(mode)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use ngterm_core::remote::viewer_run::KEEP_ALIVE;

    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// A machine whose `ssh -G` prints `config` and whose every run ends with `status`, recording what happened.
    #[derive(Default)]
    struct Fake {
        config: Option<String>,
        status: Option<i32>,
        probed: Vec<Vec<String>>,
        ran: Vec<Vec<String>>,
        printed: String,
        waited: Option<LostLink>,
    }

    impl Machine for Fake {
        fn output(&mut self, argv: &[String]) -> Option<String> {
            self.probed.push(argv.to_vec());
            self.config.clone()
        }

        fn run(&mut self, argv: &[String]) -> io::Result<i32> {
            self.ran.push(argv.to_vec());
            self.status
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "program not found"))
        }

        fn print(&mut self, text: &str) {
            self.printed.push_str(text);
        }

        fn wait_lost(&mut self, lost: &LostLink) {
            self.waited = Some(lost.clone());
        }
    }

    fn pane(lost: Option<LostLink>) -> ViewerRun {
        ViewerRun::Pane {
            attach: words(&["ssh", "-tt", "box", "tail"]),
            report: "ngterm: build (right) on box disconnected, exit".to_owned(),
            lost,
        }
    }

    fn lost() -> LostLink {
        LostLink {
            bar: "Connection to box lost".to_owned(),
            title: "ngterm-remote;n1:lost".to_owned(),
        }
    }

    #[test]
    fn any_other_command_line_passes_through() {
        let plain: Vec<OsString> = ["ngtermctl", "live", "tree"].iter().map(OsString::from).collect();
        assert!(matches!(intercept(plain.clone()), Intercepted::Arguments(got) if got == plain));
        let extra: Vec<OsString> = ["ngtermctl", VIEWER_RUN, "e30", "more"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(matches!(intercept(extra.clone()), Intercepted::Arguments(got) if got == extra));
    }

    #[test]
    fn a_word_no_build_wrote_is_a_usage_error() {
        let args: Vec<OsString> = ["ngtermctl", VIEWER_RUN, "?!"].iter().map(OsString::from).collect();
        assert!(matches!(intercept(args), Intercepted::Exit(code) if code == ExitCode::from(64)));
    }

    #[test]
    fn the_pane_keeps_the_ssh_exit_status_and_says_what_died() {
        let mut fake = Fake {
            config: Some("user me\nserveraliveinterval 0\n".to_owned()),
            status: Some(23),
            ..Fake::default()
        };
        assert_eq!(carry_out(&pane(None), &mut fake), 23);
        assert_eq!(fake.printed, "ngterm: build (right) on box disconnected, exit 23\n");
        assert_eq!(fake.probed, [words(&["ssh", "-G", "-tt", "box", "tail"])]);
        let mut with_keep_alive = words(&["ssh", "-tt"]);
        with_keep_alive.extend(KEEP_ALIVE.map(str::to_owned));
        with_keep_alive.extend(words(&["box", "tail"]));
        assert_eq!(fake.ran, [with_keep_alive], "a config with no keep-alive gets one");
    }

    #[test]
    fn a_config_with_its_own_keep_alive_runs_the_attach_as_built() {
        let mut fake = Fake {
            config: Some("serveraliveinterval 15\n".to_owned()),
            status: Some(0),
            ..Fake::default()
        };
        assert_eq!(carry_out(&pane(None), &mut fake), 0);
        assert_eq!(fake.ran, [words(&["ssh", "-tt", "box", "tail"])]);
    }

    #[test]
    fn a_lost_link_with_a_lead_shows_the_bar_and_waits() {
        let mut fake = Fake {
            status: Some(255),
            ..Fake::default()
        };
        assert_eq!(carry_out(&pane(Some(lost())), &mut fake), 255);
        assert_eq!(fake.waited, Some(lost()));
        assert_eq!(fake.printed, "", "the bar replaces the report");
        // any other exit keeps the plain line and status
        let mut fake = Fake {
            status: Some(1),
            ..Fake::default()
        };
        assert_eq!(carry_out(&pane(Some(lost())), &mut fake), 1);
        assert_eq!(fake.waited, None);
        assert_eq!(fake.printed, "ngterm: build (right) on box disconnected, exit 1\n");
    }

    #[test]
    fn without_a_lead_a_lost_link_exits_as_any_other_end() {
        let mut fake = Fake {
            status: Some(255),
            ..Fake::default()
        };
        assert_eq!(carry_out(&pane(None), &mut fake), 255);
        assert_eq!(fake.waited, None);
        assert_eq!(fake.printed, "ngterm: build (right) on box disconnected, exit 255\n");
    }

    #[test]
    fn an_ssh_that_is_not_there_reads_as_a_shells_not_found() {
        let mut fake = Fake::default();
        assert_eq!(carry_out(&pane(Some(lost())), &mut fake), 127);
        assert_eq!(fake.printed, "ngterm: build (right) on box disconnected, exit 127\n");
    }

    #[test]
    fn a_program_runs_as_given_and_ends_with_its_status() {
        let argv = words(&["ssh", "-tt", "box", "it's $(a) %c%"]);
        let mut fake = Fake {
            status: Some(5),
            ..Fake::default()
        };
        assert_eq!(carry_out(&ViewerRun::Program { argv: argv.clone() }, &mut fake), 5);
        assert_eq!(fake.ran, [argv]);
        assert!(fake.probed.is_empty());
        assert_eq!(fake.printed, "");
    }

    /// The real machine runs a program on this stdio and hands back its status.
    #[test]
    fn the_process_machine_carries_a_programs_status() {
        let argv = if cfg!(windows) {
            words(&["cmd.exe", "/d", "/c", "exit 7"])
        } else {
            words(&["/bin/sh", "-c", "exit 7"])
        };
        assert_eq!(carry_out(&ViewerRun::Program { argv }, &mut Process), 7);
        assert_eq!(exit_code(300), ExitCode::from(1));
    }

    /// Through `cmd.exe`, which runs a Windows pane's command line, the line runs the helper with its one word intact.
    #[cfg(windows)]
    #[test]
    fn cmd_hands_the_word_over_intact() {
        let run = pane(Some(lost()));
        let line = run.line();
        let word = line.rsplit(' ').next().unwrap().to_owned();
        let output = Command::new("cmd.exe")
            .args(["/d", "/c"])
            .arg(format!("echo {word}"))
            .output()
            .unwrap();
        let echoed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert_eq!(ViewerRun::decode(&echoed).unwrap(), run);
    }
}
