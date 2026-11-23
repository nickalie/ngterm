//! launchd's side of a server's job: its plist, `launchctl` and what the system reports.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::{Job, Outcome};

const LAUNCHCTL: &str = "/bin/launchctl";

/// The job's definition: interactive, run once at load, no environment of the client's (a session's travels in
/// its request).
///
/// # Errors
///
/// An argument is not UTF-8, which a property list cannot hold.
pub(super) fn definition(job: &Job) -> io::Result<Vec<u8>> {
    let arguments = job
        .arguments
        .iter()
        .map(|argument| {
            argument
                .to_str()
                .map(|text| plist::Value::String(text.to_owned()))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{} is not UTF-8", argument.to_string_lossy()),
                    )
                })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let definition = plist::Dictionary::from_iter([
        ("Label", plist::Value::String(job.label.clone())),
        ("ProgramArguments", plist::Value::Array(arguments)),
        ("RunAtLoad", plist::Value::Boolean(true)),
        ("KeepAlive", plist::Value::Boolean(false)),
        ("ProcessType", plist::Value::String("Interactive".to_owned())),
        ("AbandonProcessGroup", plist::Value::Boolean(true)),
        ("Umask", plist::Value::Integer(job.umask.into())),
    ]);
    let mut xml = Vec::new();
    plist::Value::Dictionary(definition)
        .to_writer_xml(&mut xml)
        .map_err(io::Error::other)?;
    Ok(xml)
}

/// Loads the job from a private plist in `directory`, removed once `launchctl` returns: launchd keeps the
/// definition it read.
pub(super) fn bootstrap(job: &Job, directory: &Path, deadline: Instant) -> Outcome {
    let path = directory.join(format!("{}.plist", job.label));
    if let Err(error) = write_plist(job, &path) {
        let _ = fs::remove_file(&path);
        return Outcome::Failed(format!("plist: {error}"));
    }
    let outcome = launchctl(
        &[
            OsString::from("bootstrap"),
            OsString::from(domain()),
            path.as_os_str().to_owned(),
        ],
        deadline,
    );
    let _ = fs::remove_file(&path);
    outcome
}

fn write_plist(job: &Job, path: &Path) -> io::Result<()> {
    let xml = definition(job)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(&xml)
}

/// Removes the job `label`, which ends its process when it still runs.
pub(super) fn bootout(label: &str, deadline: Instant) -> Outcome {
    launchctl(
        &[
            OsString::from("bootout"),
            OsString::from(format!("{}/{label}", domain())),
        ],
        deadline,
    )
}

pub(super) fn product_version() -> Option<String> {
    let mut buffer = [0_u8; 32];
    let mut length = buffer.len();
    // SAFETY: the name is NUL-terminated, the buffer is writable for `length` bytes, and no new value is set.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            buffer.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    let text = buffer.get(..length)?;
    let text = text.split(|byte| *byte == 0).next()?;
    String::from_utf8(text.to_vec()).ok()
}

pub(super) fn current_umask() -> u32 {
    // SAFETY: umask takes no pointers; the mask read is put back at once.
    let previous = unsafe { libc::umask(0o022) };
    // SAFETY: as above.
    unsafe { libc::umask(previous) };
    u32::from(previous)
}

/// The user's GUI domain.
fn domain() -> String {
    let uid = rustix::process::getuid().as_raw();
    format!("gui/{uid}")
}

/// Runs `launchctl` with `arguments` until `deadline`; its exit status is the error number it prints.
fn launchctl(arguments: &[OsString], deadline: Instant) -> Outcome {
    run(Path::new(LAUNCHCTL), arguments, deadline)
}

/// Runs `program`, a `launchctl`, with `arguments` until `deadline`.
fn run(program: &Path, arguments: &[OsString], deadline: Instant) -> Outcome {
    let verb = arguments
        .first()
        .map(|verb| verb.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut child = match Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Outcome::Failed(format!("launchctl: {error}")),
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Outcome::Succeeded,
            Ok(Some(status)) => return Outcome::Failed(format!("launchctl {verb} {status}")),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Uncertain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::LAUNCHCTL_BUDGET;
    use super::*;

    fn job() -> Job {
        Job::with_nonce(
            Path::new("/A.app/Contents/MacOS/ngterm-session"),
            Path::new("/tmp/live"),
            0o022,
            "deadbeef",
        )
    }

    #[test]
    fn the_plist_defines_an_interactive_one_shot_job_without_an_environment() {
        let job = job();
        let value = plist::Value::from_reader_xml(definition(&job).unwrap().as_slice()).unwrap();
        let definition = value.as_dictionary().unwrap();
        let mut keys: Vec<_> = definition.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "AbandonProcessGroup",
                "KeepAlive",
                "Label",
                "ProcessType",
                "ProgramArguments",
                "RunAtLoad",
                "Umask"
            ]
        );
        assert_eq!(definition["Label"].as_string(), Some(job.label.as_str()));
        let arguments: Vec<_> = definition["ProgramArguments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| OsString::from(argument.as_string().unwrap()))
            .collect();
        assert_eq!(arguments, job.arguments);
        assert_eq!(definition["RunAtLoad"].as_boolean(), Some(true));
        assert_eq!(definition["KeepAlive"].as_boolean(), Some(false));
        assert_eq!(definition["ProcessType"].as_string(), Some("Interactive"));
        assert_eq!(definition["AbandonProcessGroup"].as_boolean(), Some(true));
        assert_eq!(definition["Umask"].as_signed_integer(), Some(18));
    }

    #[test]
    fn a_non_utf8_argument_has_no_plist() {
        use std::os::unix::ffi::OsStrExt as _;
        let directory = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        let error = definition(&Job::new(Path::new("/x"), directory, 0o022)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn an_unwritable_plist_fails_without_loading_anything() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let outcome = bootstrap(&job(), &missing, Instant::now() + LAUNCHCTL_BUDGET);
        let Outcome::Failed(reason) = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert!(reason.starts_with("plist:"), "{reason}");
        assert!(!missing.exists());
    }

    #[test]
    fn launchctl_s_exit_status_and_deadline_decide_the_outcome() {
        let soon = || Instant::now() + LAUNCHCTL_BUDGET;
        let shell = Path::new("/bin/sh");
        let script = |body: &str| [OsString::from("-c"), OsString::from(body)];
        assert_eq!(run(shell, &script("exit 0"), soon()), Outcome::Succeeded);
        assert_eq!(
            run(shell, &script("exit 5"), soon()),
            Outcome::Failed("launchctl -c exit status: 5".to_owned())
        );
        let begun = Instant::now();
        assert_eq!(
            run(shell, &script("sleep 30"), Instant::now() + Duration::from_millis(100)),
            Outcome::Uncertain
        );
        assert!(begun.elapsed() < Duration::from_secs(10));
        let missing = run(Path::new("/nonexistent/launchctl"), &[], soon());
        assert!(
            matches!(&missing, Outcome::Failed(reason) if reason.starts_with("launchctl: ")),
            "{missing:?}"
        );
    }
}
