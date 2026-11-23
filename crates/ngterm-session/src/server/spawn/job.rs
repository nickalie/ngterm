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
use std::path::Path;
use std::time::{Duration, Instant};

/// The first part of every job's label: ngterm's app id and what the job runs.
pub const LABEL_ROOT: &str = "com.nickalie.ngterm.session-server";

/// The flag of a job's server line, `server <directory> --launchd-job <label>`.
pub const JOB_FLAG: &str = "--launchd-job";

/// The longest one `launchctl` call may take, as upstream's.
pub const LAUNCHCTL_BUDGET: Duration = Duration::from_secs(2);

/// The first macOS release whose servers start as jobs.
const FIRST_MAJOR: u32 = 27;

#[cfg(not(target_os = "macos"))]
const NO_LAUNCHD: &str = "launchd runs only on macOS";

#[cfg(target_os = "macos")]
mod macos;

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

    /// Loads the job from a private plist in `directory`, removed once `launchctl` returns: launchd keeps the
    /// definition it read. Only macOS has launchd.
    #[must_use]
    pub fn bootstrap(&self, directory: &Path, deadline: Instant) -> Outcome {
        #[cfg(target_os = "macos")]
        return macos::bootstrap(self, directory, deadline);
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (directory, deadline);
            Outcome::Failed(NO_LAUNCHD.to_owned())
        }
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
    #[cfg(target_os = "macos")]
    return macos::bootout(label, deadline);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (label, deadline);
        Outcome::Failed(NO_LAUNCHD.to_owned())
    }
}

/// This process's file mode creation mask, which a job's server keeps.
#[must_use]
pub fn current_umask() -> u32 {
    #[cfg(target_os = "macos")]
    return macos::current_umask();
    #[cfg(not(target_os = "macos"))]
    0o022
}

/// Whether this system starts servers as jobs: macOS 27 and later.
#[must_use]
pub fn supported() -> bool {
    #[cfg(target_os = "macos")]
    let version = macos::product_version();
    #[cfg(not(target_os = "macos"))]
    let version: Option<String> = None;
    version
        .as_deref()
        .and_then(major)
        .is_some_and(|major| major >= FIRST_MAJOR)
}

/// The major number of a product version such as `27.2.1`.
fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
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
                ngterm_core::live::fnv1a64(std::path::absolute("/tmp/live").unwrap().to_string_lossy().as_bytes())
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn only_macos_loads_or_removes_a_job() {
        let root = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + LAUNCHCTL_BUDGET;
        let failed = Outcome::Failed(NO_LAUNCHD.to_owned());
        assert_eq!(job().bootstrap(root.path(), deadline), failed);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        assert_eq!(bootout(&job().label, deadline), failed);
        assert_eq!(current_umask(), 0o022);
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
