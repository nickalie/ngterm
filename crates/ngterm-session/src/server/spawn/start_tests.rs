//! The start's choice between a launchd job and the spawn, on a fake starter in a namespace nobody serves.
//!
//! Upstream: `agtermCore/Tests/SessionHostRuntimeTests/ClientConnectorTests.swift` (#987).

use std::cell::RefCell;

use super::*;
use crate::protocol::Role;

/// A starter whose job ends in `outcome` and whose spawn is refused.
struct Fake {
    launchd: bool,
    outcome: job::Outcome,
    jobs: RefCell<Vec<(job::Job, PathBuf, Instant)>>,
    /// Whether the plist's directory was there at each bootstrap.
    directories: RefCell<Vec<bool>>,
    spawns: RefCell<Vec<PathBuf>>,
}

impl Fake {
    fn new(launchd: bool, outcome: job::Outcome) -> Self {
        Self {
            launchd,
            outcome,
            jobs: RefCell::default(),
            directories: RefCell::default(),
            spawns: RefCell::default(),
        }
    }
}

impl Starter for Fake {
    fn launchd_supported(&self) -> bool {
        self.launchd
    }

    fn bootstrap(&self, job: &job::Job, directory: &Path, deadline: Instant) -> job::Outcome {
        self.jobs
            .borrow_mut()
            .push((job.clone(), directory.to_path_buf(), deadline));
        self.directories.borrow_mut().push(directory.is_dir());
        self.outcome.clone()
    }

    fn spawn(&self, _executable: &Path, directory: &Path) -> io::Result<Spawned> {
        self.spawns.borrow_mut().push(directory.to_path_buf());
        Err(io::Error::other("spawn refused"))
    }
}

/// A namespace nobody serves.
struct Namespace {
    _root: tempfile::TempDir,
    endpoint: Endpoint,
}

impl Namespace {
    fn new() -> Self {
        // short on Unix, where a socket path holds about a hundred bytes
        #[cfg(unix)]
        let root = tempfile::Builder::new().prefix("ngjs-").tempdir_in("/tmp").unwrap();
        #[cfg(windows)]
        let root = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::new(root.path().join("live"));
        Self { _root: root, endpoint }
    }

    fn start(&self, starter: &Fake, timeout: Duration) -> Result<Connected, StartError> {
        self.start_with(starter, timeout, true)
    }

    fn start_with(&self, starter: &Fake, timeout: Duration, launchd: bool) -> Result<Connected, StartError> {
        let options = StartOptions {
            timeout,
            launchd,
            ..StartOptions::new("/opt/ngterm/bin/ngterm-session")
        };
        let hello = Hello {
            role: Role::Control,
            build: "start-test".to_owned(),
            capabilities: Vec::new(),
        };
        start_with(&self.endpoint, &hello, &options, starter)
    }
}

#[test]
fn a_loaded_or_uncertain_job_is_never_started_twice() {
    for outcome in [job::Outcome::Succeeded, job::Outcome::Uncertain] {
        let namespace = Namespace::new();
        let starter = Fake::new(true, outcome.clone());
        let error = namespace.start(&starter, Duration::from_millis(300)).unwrap_err();
        assert!(matches!(error, StartError::NotReady { .. }), "{outcome:?}: {error}");
        assert_eq!(starter.jobs.borrow().len(), 1, "{outcome:?}");
        assert!(starter.spawns.borrow().is_empty(), "{outcome:?}");
    }
}

#[test]
fn a_failed_job_falls_back_to_the_spawn() {
    let namespace = Namespace::new();
    let starter = Fake::new(
        true,
        job::Outcome::Failed("launchctl bootstrap exit status: 5".to_owned()),
    );
    let error = namespace.start(&starter, Duration::from_secs(10)).unwrap_err();
    assert!(matches!(error, StartError::Spawn { .. }), "{error}");
    assert_eq!(starter.jobs.borrow().len(), 1);
    assert_eq!(*starter.spawns.borrow(), [namespace.endpoint.directory()]);
}

#[test]
fn a_system_without_jobs_spawns_as_before() {
    let namespace = Namespace::new();
    let starter = Fake::new(false, job::Outcome::Succeeded);
    let error = namespace.start(&starter, Duration::from_secs(10)).unwrap_err();
    assert!(matches!(error, StartError::Spawn { .. }), "{error}");
    assert!(starter.jobs.borrow().is_empty());
    assert_eq!(*starter.spawns.borrow(), [namespace.endpoint.directory()]);
}

#[test]
fn the_job_runs_this_executable_as_the_namespaces_server() {
    let namespace = Namespace::new();
    let starter = Fake::new(true, job::Outcome::Failed("x".to_owned()));
    let _ = namespace.start(&starter, Duration::from_secs(10));
    let jobs = starter.jobs.borrow();
    let (job, directory, _) = jobs.first().unwrap();
    let namespace_directory = namespace.endpoint.directory();
    assert!(job::is_own(&job.label, namespace_directory), "{}", job.label);
    assert_eq!(
        job.arguments,
        [
            OsString::from("/opt/ngterm/bin/ngterm-session"),
            OsString::from("server"),
            namespace_directory.as_os_str().to_owned(),
            OsString::from(job::JOB_FLAG),
            OsString::from(&job.label),
        ]
    );
    assert_eq!(directory, namespace_directory);
}

#[test]
fn launchctl_gets_its_budget_cut_to_the_start_deadline() {
    for (timeout, budget) in [
        (Duration::from_secs(10), job::LAUNCHCTL_BUDGET),
        (Duration::from_millis(300), Duration::from_millis(300)),
    ] {
        let namespace = Namespace::new();
        let starter = Fake::new(true, job::Outcome::Failed("x".to_owned()));
        let begun = Instant::now();
        let _ = namespace.start(&starter, timeout);
        let ended = Instant::now();
        let deadline = starter.jobs.borrow().first().unwrap().2;
        assert!(deadline >= begun + budget, "{timeout:?}: {:?}", deadline - begun);
        assert!(deadline <= ended + budget, "{timeout:?}: {:?}", deadline - begun);
    }
}

#[test]
fn the_server_of_the_loaded_job_counts_as_started_here() {
    let welcome = |job: Option<&str>| Welcome {
        build: "b".to_owned(),
        capabilities: Vec::new(),
        pid: 42,
        error: None,
        login: None,
        job: job.map(str::to_owned),
    };
    assert_eq!(
        job_started(Some("a.1"), &welcome(Some("a.1"))),
        Some(Started {
            pid: 42,
            path: SpawnPath::LaunchdJob
        })
    );
    assert_eq!(job_started(Some("a.1"), &welcome(Some("a.2"))), None);
    assert_eq!(job_started(Some("a.1"), &welcome(None)), None);
    assert_eq!(job_started(None, &welcome(None)), None);
}

#[test]
fn the_job_finds_the_namespace_directory_made_before_any_server() {
    let namespace = Namespace::new();
    assert!(!namespace.endpoint.directory().exists());
    let starter = Fake::new(true, job::Outcome::Failed("x".to_owned()));
    let _ = namespace.start(&starter, Duration::from_secs(10));
    assert_eq!(*starter.directories.borrow(), [true]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(namespace.endpoint.directory())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
}

#[test]
fn a_caller_that_keeps_the_spawn_loads_no_job() {
    let namespace = Namespace::new();
    let starter = Fake::new(true, job::Outcome::Succeeded);
    let error = namespace
        .start_with(&starter, Duration::from_secs(10), false)
        .unwrap_err();
    assert!(matches!(error, StartError::Spawn { .. }), "{error}");
    assert!(starter.jobs.borrow().is_empty());
    assert_eq!(*starter.spawns.borrow(), [namespace.endpoint.directory()]);
}

#[test]
fn only_a_server_of_an_earlier_login_found_here_is_asked_to_retire() {
    let welcome = Welcome {
        build: "b".to_owned(),
        capabilities: Vec::new(),
        pid: 42,
        error: None,
        login: Some(100_001),
        job: None,
    };
    assert!(!stays(&welcome, false, None, Some(100_002)));
    assert!(stays(&welcome, false, Some(42), Some(100_002)), "asked once already");
    assert!(stays(&welcome, false, None, Some(100_001)), "of this login");
    assert!(stays(&welcome, false, None, None), "of an unknown login");
    assert!(stays(&welcome, true, None, Some(100_002)), "started by this call");
}

#[test]
fn a_handoff_and_the_servers_started_on_a_retired_pass_are_the_returned_calls() {
    let namespace = Namespace::new();
    namespace.endpoint.prepare_directory().unwrap();
    let _listener = namespace.endpoint.bind().unwrap();
    let started = |pid| Started {
        pid,
        path: SpawnPath::Detached,
    };
    let committed = HandoffAttempt::Committed { from: "a".to_owned() };
    let mut passes = vec![
        (9, Some(100_002), vec![started(9)], None),
        (7, Some(100_001), vec![started(5)], Some(committed.clone())),
    ];
    let mut retired = Vec::new();
    let connected = past_earlier_logins(
        Some(100_002),
        || {
            let (pid, login, spawned, handoff) = passes.pop().unwrap();
            Ok(Connected {
                stream: namespace.endpoint.connect().unwrap(),
                welcome: Welcome {
                    build: "b".to_owned(),
                    capabilities: Vec::new(),
                    pid,
                    error: None,
                    login,
                    job: None,
                },
                spawned,
                handoff,
            })
        },
        |pid| retired.push(pid),
    )
    .unwrap();
    assert_eq!(retired, [7]);
    assert_eq!(connected.welcome.pid, 9);
    assert_eq!(connected.handoff, Some(committed));
    assert_eq!(connected.spawned, [started(5), started(9)]);
}
