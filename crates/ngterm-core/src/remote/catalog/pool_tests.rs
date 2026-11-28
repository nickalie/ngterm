//! A remote window's shared ssh connections (upstream's `SshConnectionPoolTests`), over a fake runner whose masters
//! live in a set and whose checks or starts a test can hold.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::{CHANNELS_PER_CONNECTION, SshConnectionPool, hash};
use crate::remote::ssh::{exit_command, start_command};
use crate::remote::tree::{RemoteCommandResult, RemoteCommandRunner};

#[derive(Default)]
struct Masters {
    ran: Vec<Vec<String>>,
    alive: HashSet<String>,
    start_status: i32,
    hold_checks: bool,
    hold_starts: bool,
    /// A held check or start is waiting.
    held: bool,
}

#[derive(Default)]
struct Fake {
    masters: Mutex<Masters>,
    changed: Condvar,
}

impl Fake {
    fn with(edit: impl FnOnce(&mut Masters)) -> Arc<Self> {
        let fake = Arc::new(Self::default());
        edit(&mut fake.masters.lock().unwrap());
        fake
    }

    fn masters(&self) -> std::sync::MutexGuard<'_, Masters> {
        self.masters.lock().unwrap()
    }

    fn starts(&self) -> usize {
        self.masters().ran.iter().filter(|argv| argv[0] == "/bin/sh").count()
    }

    fn checks(&self) -> usize {
        self.masters()
            .ran
            .iter()
            .filter(|argv| argv.iter().any(|word| word == "check"))
            .count()
    }

    /// Waits until a held check or start waits.
    fn until_held(&self) {
        let masters = self.masters();
        let (masters, timeout) = self
            .changed
            .wait_timeout_while(masters, Duration::from_secs(10), |masters| !masters.held)
            .unwrap();
        assert!(!timeout.timed_out() && masters.held, "nothing was held");
    }

    fn open_gate(&self) {
        let mut masters = self.masters();
        masters.hold_checks = false;
        masters.hold_starts = false;
        drop(masters);
        self.changed.notify_all();
    }
}

impl RemoteCommandRunner for Fake {
    fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
        let line = argv.join(" ");
        let path = line
            .rsplit("ControlPath=")
            .next()
            .and_then(|rest| rest.split([' ', '\'']).next())
            .unwrap_or_default()
            .to_owned();
        let check = argv.iter().any(|word| word == "check");
        let mut masters = self.masters();
        masters.ran.push(argv.to_vec());
        if (check && masters.hold_checks) || (!check && argv[0] == "/bin/sh" && masters.hold_starts) {
            masters.held = true;
            self.changed.notify_all();
            masters = self
                .changed
                .wait_while(masters, |masters| masters.hold_checks || masters.hold_starts)
                .unwrap();
        }
        let status = if check {
            if masters.alive.contains(&path) { 0 } else { 255 }
        } else if argv[0] == "/bin/sh" {
            if masters.start_status == 0 {
                masters.alive.insert(path);
            }
            masters.start_status
        } else {
            0
        };
        RemoteCommandResult {
            status,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

/// Waits until `count` callers wait on starts in progress.
fn until_waiting(pool: &SshConnectionPool, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while pool.waiting() < count {
        assert!(
            std::time::Instant::now() < deadline,
            "{} of {count} callers wait",
            pool.waiting()
        );
        std::thread::yield_now();
    }
}

fn pool_for(fake: &Arc<Fake>, key: &str) -> SshConnectionPool {
    SshConnectionPool::new("box", Path::new("/tmp/ngterm-ssh-501"), key, fake.clone())
}

fn pool(fake: &Arc<Fake>) -> SshConnectionPool {
    pool_for(fake, "window-1")
}

#[test]
fn eight_users_share_a_connection_and_a_ninth_opens_another() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);

    let paths: Vec<String> = (0..9).map(|index| pool.lease(&format!("pane:{index}"))).collect();

    assert_eq!(paths[..8].iter().collect::<HashSet<_>>().len(), 1);
    assert_ne!(paths[8], paths[0]);
    assert_eq!(pool.load(), vec![CHANNELS_PER_CONNECTION, 1]);
    pool.settle();
}

#[test]
fn a_user_keeps_its_slot_and_a_released_one_frees_room() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    let first = (0..8)
        .map(|index| pool.lease(&format!("pane:{index}")))
        .collect::<Vec<_>>()[0]
        .clone();

    let again = pool.lease("pane:3");
    pool.release("pane:3");
    let newcomer = pool.lease("present:x");

    assert_eq!(again, first);
    assert_eq!(newcomer, first);
    assert_eq!(pool.load(), vec![8]);
    pool.settle();
}

#[test]
fn retain_releases_only_the_absent_users_of_its_kind() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    for user in ["pane:a", "pane:b", "present:a", "catalog"] {
        pool.lease(user);
    }

    pool.retain(&HashSet::from(["pane:a".to_owned()]), "pane:");

    assert_eq!(pool.load(), vec![3]);
    assert_eq!(pool.lease("pane:b"), pool.lease("pane:a"));
    pool.settle();
}

#[test]
fn a_cold_open_starts_each_connection_once_however_many_users_arrive() {
    let fake = Fake::with(|masters| masters.hold_starts = true);
    let pool = pool(&fake);

    for index in 0..20 {
        pool.lease(&format!("pane:{index}"));
        pool.lease(&format!("present:{index}"));
    }
    until_waiting(&pool, 35);
    fake.open_gate();
    pool.settle();

    assert_eq!(pool.load(), vec![8, 8, 8, 8, 8]);
    assert_eq!(fake.starts(), 5);
    assert_eq!(
        fake.checks(),
        5,
        "a user arriving during a start runs no check of its own"
    );
}

#[test]
fn callers_arriving_during_a_start_share_it() {
    let fake = Fake::with(|masters| masters.hold_starts = true);
    let pool = pool(&fake);
    let path = pool.lease("catalog");
    fake.until_held();

    let waiters: Vec<_> = (0..2)
        .map(|_| {
            let (pool, path) = (pool.clone(), path.clone());
            std::thread::spawn(move || pool.ready(&path))
        })
        .collect();
    until_waiting(&pool, 2);
    fake.open_gate();
    let results: Vec<bool> = waiters.into_iter().map(|waiter| waiter.join().unwrap()).collect();
    pool.settle();

    assert_eq!(results, vec![true, true]);
    assert_eq!(fake.starts(), 1);
    assert_eq!(fake.checks(), 1);
}

#[test]
fn a_live_connection_is_not_started_again_and_a_dead_one_is_replaced_once() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    let path = pool.lease("catalog");
    pool.settle();
    pool.lease("pane:a");
    pool.settle();
    let while_alive = fake.starts();

    fake.masters().alive.remove(&path);
    let replaced = pool.ready(&path);

    assert_eq!(while_alive, 1);
    assert!(replaced);
    assert_eq!(fake.starts(), 2);
}

#[test]
fn a_reattach_in_a_full_slot_whose_connection_died_brings_that_connection_back() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    let paths: Vec<String> = (0..8).map(|index| pool.lease(&format!("pane:{index}"))).collect();
    pool.settle();
    fake.masters().alive.remove(&paths[0]);
    let (_, probe) = pool.lease_once();
    pool.settle();
    let after_probe = fake.starts();

    let reattach = pool.lease("pane:3");
    pool.settle();

    assert_ne!(probe, paths[0]);
    assert_eq!(after_probe, 2);
    assert_eq!(reattach, paths[0]);
    assert!(fake.masters().alive.contains(&paths[0]));
    assert_eq!(fake.starts(), 3);
}

#[test]
fn closing_during_a_check_starts_nothing() {
    let fake = Fake::with(|masters| masters.hold_checks = true);
    let pool = pool(&fake);
    let path = pool.lease("catalog");
    fake.until_held();

    let _ = pool.close();
    fake.open_gate();
    pool.settle();

    assert_eq!(fake.starts(), 0);
    assert!(!fake.masters().alive.contains(&path));
}

#[test]
fn a_connection_that_finishes_starting_after_close_is_ended_at_once() {
    let fake = Fake::with(|masters| masters.hold_starts = true);
    let pool = pool(&fake);
    let path = pool.lease("catalog");
    fake.until_held();

    let _ = pool.close();
    fake.open_gate();
    pool.settle();

    assert_eq!(fake.starts(), 1);
    assert_eq!(fake.masters().ran.last(), Some(&exit_command("box", &path)));
}

#[test]
fn pages_jobs_and_commands_past_eight_open_another_connection() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    for index in 0..3 {
        pool.lease(&format!("page:{index}"));
        pool.lease(&format!("job:{index}"));
    }
    let once: Vec<(String, String)> = (0..3).map(|_| pool.lease_once()).collect();

    pool.release(&once[0].0);

    assert_ne!(once[2].1, once[0].1);
    assert_eq!(pool.load(), vec![7, 1]);
    pool.settle();
}

#[test]
fn closing_drops_every_lease_and_ends_each_connection() {
    let fake = Fake::with(|_| {});
    let pool = pool(&fake);
    for index in 0..9 {
        pool.lease(&format!("pane:{index}"));
    }

    let commands = pool.close();
    let after_close = pool.ready(&pool.lease("pane:0"));
    pool.settle();

    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|command| command[..3] == ["ssh", "-O", "exit"]));
    assert!(!after_close);
}

#[test]
fn a_host_ssh_cannot_take_has_no_connection() {
    let fake = Fake::with(|_| {});
    let pool = SshConnectionPool::new("-oProxyCommand=x", Path::new("/tmp/p"), "w", fake.clone());
    let path = pool.lease("catalog");
    pool.settle();
    assert!(!pool.ready(&path));
    assert!(fake.masters().ran.is_empty());
    assert!(!pool.ready("/tmp/not-a-slot"));
}

#[test]
fn paths_are_short_and_differ_per_owner() {
    let fake = Fake::with(|_| {});
    let one = pool_for(&fake, "/tmp/a|window-1");
    let other = pool_for(&fake, "/tmp/b|window-1");

    let (one_path, other_path) = (one.lease("x"), other.lease("x"));

    assert_ne!(one_path, other_path);
    assert!(one_path.len() < 60, "{one_path}");
    assert_eq!(hash("same"), hash("same"));
    assert_eq!(hash(""), "cbf29ce484222325");
    assert!(format!("{one:?}").contains("box"));
    one.settle();
    other.settle();
}

#[test]
fn the_connection_starts_detached_from_the_runners_pipes() {
    let command = start_command("box", "/tmp/p-0");

    assert_eq!(command[..2], ["/bin/sh", "-c"]);
    assert!(command[2].ends_with(" </dev/null >/dev/null 2>&1"), "{}", command[2]);
    assert!(command[2].contains("ControlPersist=60"));
    assert!(command[2].contains("'-f' '-N' '-M'"), "{}", command[2]);
}

/// A killed master leaves its socket, and `ssh -M` then runs without sharing.
#[test]
fn a_start_removes_the_socket_a_dead_connection_left() {
    let command = start_command("box", "/tmp/p-0");

    assert!(command[2].starts_with("rm -f '/tmp/p-0'; "), "{}", command[2]);
    assert_eq!(command[2].matches("; exec ").count(), 1);
}

#[cfg(unix)]
mod keep_alive {
    use std::os::unix::fs::PermissionsExt;

    use super::start_command;

    /// Runs the start command with an `ssh` that logs its calls and answers `-G` with `config`; the calls.
    fn run_start(config: &str) -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls");
        let ssh = dir.path().join("ssh");
        std::fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = -G ]; then\n{config}\nfi\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = start_command("box", &dir.path().join("p-0").display().to_string());
        let status = std::process::Command::new(&command[0])
            .args(&command[1..])
            .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Panes joined to a master ignore their own keep-alive, so a master without one never notices a dead link.
    #[test]
    fn the_shared_connection_gets_keep_alive_only_when_the_users_config_sets_none() {
        for (config, added) in [
            ("echo 'serveraliveinterval 0'", true),
            ("echo 'serveraliveinterval 15'", false),
            ("true", false),
        ] {
            let calls = run_start(config);
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert!(
                calls[0].starts_with("-G -N -o BatchMode=yes"),
                "the check sees the master's own options"
            );
            assert!(calls[1].starts_with("-f -N -M"), "{calls:?}");
            assert_eq!(
                calls[1].contains("-o ServerAliveInterval=5 -o ServerAliveCountMax=2 box"),
                added,
                "{config}: {calls:?}"
            );
        }
    }

    /// A check without `-N` reads a `Match sessiontype` block for another session type than the master's.
    #[test]
    fn the_keep_alive_check_answers_for_the_shared_connections_session_type() {
        let calls = run_start(
            r#"case " $* " in *" -N "*) echo 'serveraliveinterval 0' ;; *) echo 'serveraliveinterval 15' ;; esac"#,
        );
        assert!(
            calls[1].contains("-o ServerAliveInterval=5 -o ServerAliveCountMax=2 box"),
            "{calls:?}"
        );
    }
}
