//! On macOS 27 and later a client starts the server as a launchd job of the user's GUI domain, which the server
//! removes when it stops; a second job for the same namespace loses it and removes only its own job. Each test runs in
//! a private namespace, skips on earlier macOS, and boots out every job of its namespace at its end.
//!
//! Upstream: `agtermTests/SessionHostClientTests.swift` (`hostJobs`, `testJobHostThatFindsTheEndpointTakenRemovesOnlyItsOwnJob`, #987).
#![cfg(target_os = "macos")]

#[expect(
    dead_code,
    reason = "the helpers serve the pane, lead, snapshot, attach and server tests alike"
)]
mod support;

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_session::endpoint::{Endpoint, Stream};
use ngterm_session::protocol::{Control, Hello, RetireResult, Role, read_frame, write_control};
use ngterm_session::server::spawn::{Connected, SpawnPath, StartOptions, connect_or_start, job};
use support::{PATIENCE, TestResult, exe};

/// A private namespace whose jobs are booted out at its end.
struct Namespace {
    _root: tempfile::TempDir,
    endpoint: Endpoint,
}

impl Namespace {
    fn new() -> TestResult<Self> {
        let root = tempfile::Builder::new().prefix("ngld-").tempdir_in("/tmp")?;
        let endpoint = Endpoint::new(root.path().join("live"));
        Ok(Self { _root: root, endpoint })
    }

    fn directory(&self) -> &Path {
        self.endpoint.directory()
    }

    fn start(&self) -> TestResult<Connected> {
        let hello = Hello {
            role: Role::Control,
            build: "launchd-test".to_owned(),
            capabilities: Vec::new(),
        };
        Ok(connect_or_start(&self.endpoint, &hello, &StartOptions::new(exe()))?)
    }

    /// The loaded jobs of this namespace, sorted.
    fn jobs(&self) -> TestResult<Vec<String>> {
        let output = Command::new("/bin/launchctl")
            .arg("list")
            .stderr(Stdio::null())
            .output()?;
        let prefix = job::label_prefix(self.directory());
        let mut jobs: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split('\t').next_back())
            .filter(|label| label.starts_with(&prefix))
            .map(str::to_owned)
            .collect();
        jobs.sort();
        Ok(jobs)
    }

    /// Waits until this namespace's jobs are `expected`.
    fn wait_jobs(&self, expected: &[String]) -> TestResult<Vec<String>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let jobs = self.jobs()?;
            if jobs == expected || Instant::now() >= deadline {
                return Ok(jobs);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        for label in self.jobs().unwrap_or_default() {
            let _ = job::bootout(&label, Instant::now() + job::LAUNCHCTL_BUDGET);
        }
    }
}

/// Asks the server on `stream` to retire, which it does as it holds no session.
fn retire(mut stream: Stream) -> TestResult {
    write_control(&mut stream, &Control::RetireRequest)?;
    let answer = read_frame(&mut stream)?.control()?;
    assert!(
        matches!(answer, Control::RetireResult(RetireResult(Ok(())))),
        "{answer:?}"
    );
    Ok(())
}

fn alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 takes no pointers and sends nothing.
    i32::try_from(pid).is_ok_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

fn skip() -> bool {
    let skip = !job::supported();
    if skip {
        eprintln!("skipped: servers start as launchd jobs on macOS 27 and later");
    }
    skip
}

#[test]
fn the_server_starts_as_a_launchd_job_and_removes_it_when_it_stops() -> TestResult {
    if skip() {
        return Ok(());
    }
    let namespace = Namespace::new()?;
    let connected = namespace.start()?;
    let started = connected.started_here().expect("this call started the server");
    assert_eq!(started.path, SpawnPath::LaunchdJob);
    let label = connected.welcome.job.clone().expect("the welcome names the job");
    assert!(job::is_own(&label, namespace.directory()), "{label}");
    assert_eq!(namespace.jobs()?, [label.clone()]);
    assert!(
        connected
            .welcome
            .capabilities
            .iter()
            .any(|capability| capability == ngterm_session::server::CAPABILITY_DETACHED),
        "a job's server is detached: {:?}",
        connected.welcome.capabilities
    );
    let pid = connected.welcome.pid;
    retire(connected.stream)?;
    assert_eq!(namespace.wait_jobs(&[])?, Vec::<String>::new());
    let deadline = Instant::now() + PATIENCE;
    while alive(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the job's server stopped");
    Ok(())
}

#[test]
fn a_second_job_of_the_namespace_removes_only_its_own() -> TestResult {
    if skip() {
        return Ok(());
    }
    let namespace = Namespace::new()?;
    let connected = namespace.start()?;
    let first = namespace.jobs()?;
    assert_eq!(first.len(), 1, "{first:?}");
    let second = job::Job::new(Path::new(exe()), namespace.directory(), job::current_umask());
    let outcome = second.bootstrap(namespace.directory(), Instant::now() + job::LAUNCHCTL_BUDGET);
    assert_eq!(outcome, job::Outcome::Succeeded);
    assert_eq!(
        namespace.wait_jobs(&first)?,
        first,
        "the second job lost the namespace and removed itself"
    );
    let again = namespace.start()?;
    assert_eq!(again.welcome.pid, connected.welcome.pid);
    assert!(again.started_here().is_none());
    drop(again);
    retire(connected.stream)?;
    assert_eq!(namespace.wait_jobs(&[])?, Vec::<String>::new());
    Ok(())
}
