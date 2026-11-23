//! A server started as a one-shot launchd job in the user's GUI domain, on macOS 27 and later: the job gives the
//! server and its sessions a coalition of their own, where a spawned server stays in the app's, so a force-quit of a
//! program started in a session (one switched off in Allow in the Background) reaches only that program. A definite
//! bootstrap failure leaves the start to the spawn; one whose outcome is unknown starts nothing more, since a second
//! server would race the job for the namespace. The server removes its own job however serving ends
//! ([`super::super::command`]); a server of a job not its namespace's refuses to serve.
//!
//! Upstream: `agtermCore/Sources/SessionHostRuntime/HostJob.swift` and `ClientConnector.swift` (#987). Changed: the
//! label names ngterm's app id and the namespace directory, as one server serves a namespace here, and the job runs
//! `ngterm-session server <directory> --launchd-job <label>`.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The first part of every job's label: ngterm's app id and what the job runs.
pub const LABEL_ROOT: &str = "com.nickalie.ngterm.session-server";

/// The flag of a job's server line, `server <directory> --launchd-job <label>`.
pub const JOB_FLAG: &str = "--launchd-job";

/// The longest one `launchctl` call may take, as upstream's.
pub const LAUNCHCTL_BUDGET: Duration = Duration::from_secs(2);

/// The first macOS release whose servers start as jobs.
const FIRST_MAJOR: u32 = 27;

const LAUNCHCTL: &str = "/bin/launchctl";

/// How a `launchctl` call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed(String),
    /// `launchctl` outlived its deadline and was killed: the job may or may not be loaded.
    Uncertain,
}

/// One attempt's job for a namespace's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub label: String,
    pub arguments: Vec<OsString>,
    pub umask: u32,
}

impl Job {
    /// The job that runs `executable` as the server of `directory`, with a nonce of its own so that a new attempt
    /// never replaces or removes a job another attempt loaded.
    #[must_use]
    pub fn new(executable: &Path, directory: &Path, umask: u32) -> Self {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        Self::with_nonce(executable, directory, umask, &nonce[..8])
    }

    #[must_use]
    pub fn with_nonce(executable: &Path, directory: &Path, umask: u32, nonce: &str) -> Self {
        let label = format!("{}{nonce}", label_prefix(directory));
        let arguments = vec![
            executable.as_os_str().to_owned(),
            OsString::from("server"),
            directory.as_os_str().to_owned(),
            OsString::from(JOB_FLAG),
            OsString::from(&label),
        ];
        Self {
            label,
            arguments,
            umask,
        }
    }

    /// The job's definition: interactive, run once at load, no environment of the client's (a session's travels in
    /// its request).
    ///
    /// # Errors
    ///
    /// An argument is not UTF-8, which a property list cannot hold.
    pub fn plist(&self) -> io::Result<Vec<u8>> {
        let arguments = self
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
            ("Label", plist::Value::String(self.label.clone())),
            ("ProgramArguments", plist::Value::Array(arguments)),
            ("RunAtLoad", plist::Value::Boolean(true)),
            ("KeepAlive", plist::Value::Boolean(false)),
            ("ProcessType", plist::Value::String("Interactive".to_owned())),
            ("AbandonProcessGroup", plist::Value::Boolean(true)),
            ("Umask", plist::Value::Integer(self.umask.into())),
        ]);
        let mut xml = Vec::new();
        plist::Value::Dictionary(definition)
            .to_writer_xml(&mut xml)
            .map_err(io::Error::other)?;
        Ok(xml)
    }

    /// Loads the job from a private plist in `directory`, removed once `launchctl` returns: launchd keeps the
    /// definition it read.
    #[must_use]
    pub fn bootstrap(&self, directory: &Path, deadline: Instant) -> Outcome {
        let path = directory.join(format!("{}.plist", self.label));
        if let Err(error) = self.write_plist(&path) {
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

    fn write_plist(&self, path: &Path) -> io::Result<()> {
        let xml = self.plist()?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(path)?.write_all(&xml)
    }
}

/// What every label of `directory`'s jobs starts with: [`LABEL_ROOT`] and a hash of the absolute directory.
#[must_use]
pub fn label_prefix(directory: &Path) -> String {
    let directory = std::path::absolute(directory).unwrap_or_else(|_| directory.to_path_buf());
    let key = ngterm_core::live::fnv1a64(directory.to_string_lossy().as_bytes());
    format!("{LABEL_ROOT}.{key}.")
}

/// Whether `label` names a job of `directory`'s server: its prefix and a nonce of letters and digits.
#[must_use]
pub fn is_own(label: &str, directory: &Path) -> bool {
    label
        .strip_prefix(&label_prefix(directory))
        .is_some_and(|nonce| !nonce.is_empty() && nonce.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

/// Removes the job `label`, which ends its process when it still runs.
#[must_use]
pub fn bootout(label: &str, deadline: Instant) -> Outcome {
    launchctl(
        &[
            OsString::from("bootout"),
            OsString::from(format!("{}/{label}", domain())),
        ],
        deadline,
    )
}

/// Whether this system starts servers as jobs: macOS 27 and later.
#[must_use]
pub fn supported() -> bool {
    product_version()
        .as_deref()
        .and_then(major)
        .is_some_and(|major| major >= FIRST_MAJOR)
}

/// The major number of a product version such as `27.2.1`.
fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

#[cfg(target_os = "macos")]
fn product_version() -> Option<String> {
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

#[cfg(not(target_os = "macos"))]
fn product_version() -> Option<String> {
    None
}

/// This process's file mode creation mask, which a job's server keeps.
#[cfg(target_os = "macos")]
#[must_use]
pub fn current_umask() -> u32 {
    // SAFETY: umask takes no pointers; the mask read is put back at once.
    let previous = unsafe { libc::umask(0o022) };
    // SAFETY: as above.
    unsafe { libc::umask(previous) };
    u32::from(previous)
}

#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn current_umask() -> u32 {
    0o022
}

/// The user's GUI domain.
fn domain() -> String {
    #[cfg(unix)]
    let uid = rustix::process::getuid().as_raw();
    #[cfg(not(unix))]
    let uid = 0;
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
    fn the_label_names_the_namespace_and_the_attempt() {
        let job = job();
        let prefix = label_prefix(Path::new("/tmp/live"));
        assert_eq!(
            prefix,
            format!(
                "com.nickalie.ngterm.session-server.{}.",
                ngterm_core::live::fnv1a64(b"/tmp/live")
            )
        );
        assert_eq!(job.label, format!("{prefix}deadbeef"));
        assert_ne!(label_prefix(Path::new("/tmp/other")), prefix);
    }

    #[test]
    fn each_attempt_gets_its_own_label() {
        let first = Job::new(Path::new("/x"), Path::new("/tmp/live"), 0o022);
        let second = Job::new(Path::new("/x"), Path::new("/tmp/live"), 0o022);
        assert_ne!(first.label, second.label);
        assert_eq!(first.label.len(), label_prefix(Path::new("/tmp/live")).len() + 8);
        assert!(is_own(&first.label, Path::new("/tmp/live")));
    }

    #[test]
    fn the_arguments_carry_the_label_the_server_checks() {
        let job = job();
        assert_eq!(
            job.arguments,
            [
                "/A.app/Contents/MacOS/ngterm-session",
                "server",
                "/tmp/live",
                JOB_FLAG,
                job.label.as_str()
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn only_a_label_of_the_namespace_with_a_nonce_is_its_own() {
        let directory = Path::new("/tmp/live");
        let prefix = label_prefix(directory);
        assert!(is_own(&format!("{prefix}deadbeef"), directory));
        assert!(!is_own(&prefix, directory));
        assert!(!is_own(&format!("{prefix}dead.beef"), directory));
        assert!(!is_own(
            &format!("{}deadbeef", label_prefix(Path::new("/tmp/other"))),
            directory
        ));
        assert!(!is_own("com.example.other.deadbeef", directory));
    }

    #[test]
    fn the_plist_defines_an_interactive_one_shot_job_without_an_environment() {
        let job = job();
        let value = plist::Value::from_reader_xml(job.plist().unwrap().as_slice()).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_argument_has_no_plist() {
        use std::os::unix::ffi::OsStrExt as _;
        let directory = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        let error = Job::new(Path::new("/x"), directory, 0o022).plist().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn an_unwritable_plist_fails_without_loading_anything() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let outcome = job().bootstrap(&missing, Instant::now() + LAUNCHCTL_BUDGET);
        let Outcome::Failed(reason) = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert!(reason.starts_with("plist:"), "{reason}");
        assert!(!missing.exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_bootstrap_without_launchctl_fails_and_leaves_no_plist() {
        let root = tempfile::tempdir().unwrap();
        let outcome = job().bootstrap(root.path(), Instant::now() + LAUNCHCTL_BUDGET);
        let Outcome::Failed(reason) = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert!(reason.starts_with("launchctl: "), "{reason}");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
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

    #[test]
    fn a_major_version_is_the_first_number() {
        assert_eq!(major("27.2.1"), Some(27));
        assert_eq!(major("26"), Some(26));
        assert_eq!(major("x.1"), None);
        assert_eq!(major(""), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn no_other_system_starts_a_job() {
        assert!(!supported());
    }
}
