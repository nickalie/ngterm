//! `ngtermctl origin-run WORD`, not a user's command: what a viewer's ssh line runs on a Windows origin, whichever of
//! `cmd.exe` and PowerShell the account's sshd runs it through ([`ngterm_core::remote::origin`]). It decodes the
//! `WORD` into an [`OriginRun`] and carries it out. A `Cli` run replaces this process's own arguments, so nothing
//! stands between sshd and the stream the command carries.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use ngterm_core::remote::origin::{ORIGIN_RUN, OriginRun};

/// The largest clipboard image a [`OriginRun::Paste`] stores, as the viewer's own limit.
const MAX_PASTE: u64 = 64 * 1024 * 1024;

/// What `main` does with its arguments.
pub enum Intercepted {
    /// Parse these as the command line.
    Arguments(Vec<OsString>),
    /// Finished with this status.
    Exit(ExitCode),
}

/// `args` (the program name first) unless they ask for an [`ORIGIN_RUN`], which is carried out here.
#[must_use]
pub fn intercept(args: Vec<OsString>) -> Intercepted {
    let Some(word) = run_word(&args) else {
        return Intercepted::Arguments(args);
    };
    let run = match word.to_str().ok_or_else(|| format!("{ORIGIN_RUN}: not text")) {
        Ok(word) => OriginRun::decode(word),
        Err(error) => Err(error),
    };
    match run {
        Ok(OriginRun::Cli { words }) => {
            let mut arguments = vec![args[0].clone()];
            arguments.extend(words.into_iter().map(OsString::from));
            Intercepted::Arguments(arguments)
        }
        Ok(run) => Intercepted::Exit(carry_out(&run, &mut io::stdin().lock(), &mut io::stdout().lock())),
        Err(error) => {
            eprintln!("ngtermctl: {error}");
            Intercepted::Exit(ExitCode::from(64))
        }
    }
}

/// The `WORD` of `ngtermctl origin-run WORD`.
fn run_word(args: &[OsString]) -> Option<&OsStr> {
    match args {
        [_, command, word] if command == ORIGIN_RUN => Some(word),
        _ => None,
    }
}

fn carry_out(run: &OriginRun, input: &mut impl Read, output: &mut impl Write) -> ExitCode {
    let done = match run {
        OriginRun::Cli { .. } => Ok(ExitCode::SUCCESS),
        OriginRun::Program {
            program,
            environment,
            words,
        } => run_program(program, environment, words),
        OriginRun::Hold { ready } => hold(ready, input, output).map(|()| ExitCode::SUCCESS),
        OriginRun::Paste { extension } => {
            store_paste(&std::env::temp_dir(), extension, input, output).map(|_| ExitCode::SUCCESS)
        }
    };
    done.unwrap_or_else(|error| {
        eprintln!("ngtermctl: {error}");
        ExitCode::from(1)
    })
}

/// Runs `program` on this process's stdio and answers with its status. An empty value removes the variable.
fn run_program(program: &str, environment: &[(String, String)], words: &[String]) -> io::Result<ExitCode> {
    let mut command = Command::new(program);
    command
        .args(words)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (name, value) in environment {
        if value.is_empty() {
            command.env_remove(name);
        } else {
            command.env(name, value);
        }
    }
    let status = command.status()?;
    Ok(ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)))
}

/// Prints `ready` as a line, then reads `input` to its end.
fn hold(ready: &str, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "{ready}")?;
    output.flush()?;
    io::copy(input, &mut io::sink()).map(drop)
}

/// Stores `input` in a fresh directory under `base` (the account's own `%TEMP%` on Windows) as `clipboard.EXTENSION`
/// and prints the file's path, with no end of line. The directory outlives this process, for the agent the path is
/// pasted to; on a failure it is removed.
fn store_paste(base: &Path, extension: &str, input: &mut impl Read, output: &mut impl Write) -> io::Result<PathBuf> {
    if extension.is_empty() || !extension.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(io::Error::other("invalid image format"));
    }
    let directory = tempfile::Builder::new()
        .prefix("ngterm-paste.")
        .rand_bytes(10)
        .tempdir_in(base)?;
    let path = directory.path().join(format!("clipboard.{extension}"));
    let mut file = std::fs::File::create(&path)?;
    let copied = io::copy(&mut input.by_ref().take(MAX_PASTE + 1), &mut file)?;
    if copied > MAX_PASTE {
        return Err(io::Error::other("the image exceeds 64 MiB"));
    }
    drop(file);
    write!(output, "{}", path.display())?;
    output.flush()?;
    let _ = directory.keep();
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn word(run: &OriginRun) -> String {
        run.tail().rsplit(' ').next().unwrap().to_owned()
    }

    #[test]
    fn any_other_command_line_passes_through() {
        let plain = args(&["ngtermctl", "live", "tree"]);
        assert!(matches!(intercept(plain.clone()), Intercepted::Arguments(got) if got == plain));
        // a word that isn't alone stays clap's to refuse
        let extra = args(&["ngtermctl", ORIGIN_RUN, "e30", "more"]);
        assert!(matches!(intercept(extra.clone()), Intercepted::Arguments(got) if got == extra));
    }

    #[test]
    fn a_cli_run_becomes_the_command_line_whatever_its_words_hold() {
        let awkward = "it's $(a) \"b\" %c% é";
        let run = OriginRun::Cli {
            words: vec!["session".into(), format!("--name={awkward}")],
        };
        let intercepted = intercept(args(&["ngtermctl.exe", ORIGIN_RUN, &word(&run)]));
        let expected = args(&["ngtermctl.exe", "session", &format!("--name={awkward}")]);
        assert!(matches!(intercepted, Intercepted::Arguments(got) if got == expected));
    }

    #[test]
    fn a_word_no_build_wrote_is_a_usage_error() {
        assert!(matches!(
            intercept(args(&["ngtermctl", ORIGIN_RUN, "?!"])),
            Intercepted::Exit(code) if code == ExitCode::from(64)
        ));
    }

    /// An input that is at its end at once, and on its first read records what had been written by then.
    struct RecordingEnd<'a> {
        written: &'a std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
        seen: Option<Vec<u8>>,
    }

    impl Read for RecordingEnd<'_> {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            self.seen.get_or_insert_with(|| self.written.borrow().clone());
            Ok(0)
        }
    }

    struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn hold_prints_its_line_before_it_waits_for_the_end_of_input() {
        let written = std::rc::Rc::default();
        let mut input = RecordingEnd {
            written: &written,
            seen: None,
        };
        hold(
            "ngterm-tunnel-ready",
            &mut input,
            &mut Shared(std::rc::Rc::clone(&written)),
        )
        .unwrap();
        let seen = input.seen.expect("hold reads its input");
        assert_eq!(String::from_utf8(seen).unwrap().trim_end(), "ngterm-tunnel-ready");
    }

    #[test]
    fn a_paste_is_stored_whole_and_only_its_path_is_printed() {
        let base = tempfile::tempdir().unwrap();
        let mut output = Vec::new();
        let bytes = b"\x89PNG\r\n\x1a\n\0\xff";
        let path = store_paste(base.path(), "png", &mut &bytes[..], &mut output).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(String::from_utf8(output).unwrap(), path.display().to_string());
        let directory = path.parent().unwrap().file_name().unwrap().to_str().unwrap();
        let id = directory.strip_prefix("ngterm-paste.").unwrap();
        assert_eq!(id.len(), 10, "{directory}");
        assert_eq!(path.file_name().unwrap(), "clipboard.png");
    }

    #[test]
    fn a_paste_in_a_format_with_path_characters_stores_nothing() {
        let base = tempfile::tempdir().unwrap();
        for extension in ["", "png/../x", r"a\b"] {
            assert!(store_paste(base.path(), extension, &mut &b"x"[..], &mut Vec::new()).is_err());
        }
        assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_program_run_carries_its_environment_and_status() {
        let (program, script) = if cfg!(windows) {
            (
                "cmd.exe",
                vec![
                    "/d".to_owned(),
                    "/c".to_owned(),
                    "if \"%KEPT%\"==\"yes\" if not defined USERPROFILE exit 7".to_owned(),
                ],
            )
        } else {
            (
                "/bin/sh",
                vec![
                    "-c".to_owned(),
                    "[ \"$KEPT\" = yes ] && [ -z \"${HOME+x}\" ] && exit 7".to_owned(),
                ],
            )
        };
        // inherited here, so its absence in the child shows the empty value removed it
        let gone = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let environment = [("KEPT".to_owned(), "yes".to_owned()), (gone.to_owned(), String::new())];
        let status = run_program(program, &environment, &script).unwrap();
        assert_eq!(status, ExitCode::from(7));
    }

    /// Through each shell Windows' sshd may hand the command to, a word with the awkward characters is one argument
    /// that decodes to the original: nothing a user typed is left for a shell to read.
    #[cfg(windows)]
    #[test]
    fn the_word_is_one_plain_argument_under_cmd_and_powershell() {
        let run = OriginRun::Cli {
            words: vec!["--name=it's $(a) \"b\" %c% ^d & é".to_owned()],
        };
        let word = run.tail().rsplit(' ').next().unwrap().to_owned();
        let shells: [(&str, &[&str]); 2] = [
            ("cmd.exe", &["/d", "/c"]),
            ("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command"]),
        ];
        for (shell, flags) in shells {
            let line = format!("echo {word}");
            let output = Command::new(shell).args(flags).arg(&line).output().unwrap();
            let echoed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            assert_eq!(echoed, word, "{shell}");
            assert_eq!(OriginRun::decode(&echoed).unwrap(), run, "{shell}");
        }
    }
}
