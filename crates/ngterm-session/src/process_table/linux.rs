//! The Linux process table over `/proc` ([`crate::procfs`]'s parsers): `cmdline` for an argv (zombies and kernel
//! threads read empty and have none), `stat` for the ids and the terminal's foreground group, a scan of every `stat`
//! for a process group's members (Linux has no per-group listing), the `cwd` link for the working directory.

use std::cell::OnceCell;
use std::fs;
use std::path::{Path, PathBuf};

use ngterm_host::{ProcessStat, ProcessTable};

use crate::procfs;

/// The process table read from a procfs mount: `/proc`, or a fake tree in tests.
#[derive(Debug, Clone)]
pub struct ProcTable {
    root: PathBuf,
}

impl Default for ProcTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcTable {
    /// The live process table at `/proc`.
    #[must_use]
    pub fn new() -> Self {
        Self::at("/proc")
    }

    /// A process table laid out like `/proc` under `root`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn process_file(&self, pid: i32, name: &str) -> PathBuf {
        self.root.join(pid.to_string()).join(name)
    }

    fn read_stat(path: &Path) -> Option<ProcessStat> {
        procfs::parse_stat(&fs::read(path).ok()?)
    }

    /// Every readable process's ids, by pid: the scan of every `stat` that [`ProcessTable::group`] filters.
    fn listing(&self) -> Vec<ProcessStat> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut listed: Vec<ProcessStat> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_str().is_some_and(procfs::is_pid))
            .filter_map(|entry| Self::read_stat(&entry.path().join("stat")))
            .collect();
        listed.sort_unstable_by_key(|stat| stat.pid);
        listed
    }
}

/// The members of group `pgrp` in `listed`, by pid.
fn group_in(listed: &[ProcessStat], pgrp: i32) -> Vec<ProcessStat> {
    listed.iter().copied().filter(|stat| stat.pgrp == pgrp).collect()
}

impl ProcessTable for ProcTable {
    fn argv(&self, pid: i32) -> Option<Vec<String>> {
        procfs::parse_cmdline(&fs::read(self.process_file(pid, "cmdline")).ok()?)
    }

    fn stat(&self, pid: i32) -> Option<ProcessStat> {
        Self::read_stat(&self.process_file(pid, "stat"))
    }

    fn group(&self, pgrp: i32) -> Vec<ProcessStat> {
        group_in(&self.listing(), pgrp)
    }

    /// The `tpgid` of the stat line; `None` for a process without a terminal.
    fn terminal_foreground(&self, pid: i32) -> Option<i32> {
        procfs::parse_record(&fs::read(self.process_file(pid, "stat")).ok()?)
            .map(|record| record.foreground)
            .filter(|group| *group > 0)
    }

    fn working_directory(&self, pid: i32) -> Option<String> {
        let link = fs::read_link(self.process_file(pid, "cwd")).ok()?;
        link.into_os_string().into_string().ok()
    }

    /// The `exe` link; `None` for an image an upgrade deleted.
    fn executable_path(&self, pid: i32) -> Option<String> {
        let link = fs::read_link(self.process_file(pid, "exe")).ok()?;
        let link = link.into_os_string().into_string().ok()?;
        procfs::live_image(&link).map(str::to_owned)
    }

    fn search_path(&self, pid: i32) -> Option<String> {
        let environ = fs::read(self.process_file(pid, "environ")).ok()?;
        super::search_path_of(&procfs::parse_environ(&environ), false)
    }

    /// A view that scans every `stat` one time, at its first [`ProcessTable::group`], and filters that scan for each
    /// later one: the `tree` reads every `--command` pane's group, and a scan for each held the main thread for up to a
    /// second with a few hundred processes running.
    fn shared_listing(&self) -> Option<Box<dyn ProcessTable + '_>> {
        Some(Box::new(SharedListing {
            table: self,
            listed: OnceCell::new(),
        }))
    }
}

/// [`ProcTable`]'s [`ProcessTable::shared_listing`] view: its own reads, but one scan of every `stat` for all groups.
struct SharedListing<'a> {
    table: &'a ProcTable,
    /// Every process's ids, once scanned.
    listed: OnceCell<Vec<ProcessStat>>,
}

impl ProcessTable for SharedListing<'_> {
    fn argv(&self, pid: i32) -> Option<Vec<String>> {
        self.table.argv(pid)
    }

    fn stat(&self, pid: i32) -> Option<ProcessStat> {
        self.table.stat(pid)
    }

    fn group(&self, pgrp: i32) -> Vec<ProcessStat> {
        group_in(self.listed.get_or_init(|| self.table.listing()), pgrp)
    }

    fn terminal_foreground(&self, pid: i32) -> Option<i32> {
        self.table.terminal_foreground(pid)
    }

    fn working_directory(&self, pid: i32) -> Option<String> {
        self.table.working_directory(pid)
    }

    fn executable_path(&self, pid: i32) -> Option<String> {
        self.table.executable_path(pid)
    }

    fn search_path(&self, pid: i32) -> Option<String> {
        self.table.search_path(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(pid: i32, parent: i32, pgrp: i32, session: i32) -> ProcessStat {
        ProcessStat {
            pid,
            ppid: parent,
            pgrp,
            session,
        }
    }

    #[test]
    fn a_fake_tree_answers_argv_stat_and_group() {
        let root = tempfile::tempdir().unwrap();
        let write = |pid: i32, cmdline: &[u8], stat: &[u8]| {
            let dir = root.path().join(pid.to_string());
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("cmdline"), cmdline).unwrap();
            fs::write(dir.join("stat"), stat).unwrap();
        };
        write(100, b"-zsh\0", b"100 (zsh) S 9 100 100 0");
        write(101, b"tail\0-f\0log\0", b"101 (tail) S 100 101 100 0");
        write(102, b"grep\0x\0", b"102 (gr\xc3) S 100 101 100 0");
        write(103, b"", b"103 (defunct) Z 100 101 100 0");
        fs::create_dir(root.path().join("self")).unwrap();
        let table = ProcTable::at(root.path());

        assert_eq!(
            table.argv(101),
            Some(vec!["tail".to_owned(), "-f".to_owned(), "log".to_owned()])
        );
        assert_eq!(table.argv(103), None);
        assert_eq!(table.argv(999), None);
        assert_eq!(table.stat(102), Some(stat(102, 100, 101, 100)));
        assert_eq!(table.stat(999), None);
        assert_eq!(
            table.group(101),
            [
                stat(101, 100, 101, 100),
                stat(102, 100, 101, 100),
                stat(103, 100, 101, 100)
            ]
        );
        assert!(table.group(555).is_empty());
        assert!(ProcTable::at(root.path().join("missing")).group(101).is_empty());
    }

    #[test]
    fn a_shared_listing_scans_every_stat_one_time_for_all_groups() {
        let root = tempfile::tempdir().unwrap();
        let write = |pid: i32, cmdline: &[u8], stat: &[u8]| {
            let dir = root.path().join(pid.to_string());
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("cmdline"), cmdline).unwrap();
            fs::write(dir.join("stat"), stat).unwrap();
        };
        write(100, b"-zsh\0", b"100 (zsh) S 9 100 100 0");
        write(101, b"tail\0", b"101 (tail) S 100 101 100 0");
        write(200, b"sh\0", b"200 (sh) S 9 200 200 0");
        let table = ProcTable::at(root.path());
        let view = table.shared_listing().unwrap();

        assert_eq!(view.group(101), [stat(101, 100, 101, 100)]);
        write(102, b"grep\0", b"102 (grep) S 100 101 100 0");
        write(201, b"cat\0", b"201 (cat) S 200 200 200 0");
        assert_eq!(
            view.group(101),
            [stat(101, 100, 101, 100)],
            "the first scan serves the pass"
        );
        assert_eq!(
            view.group(200),
            [stat(200, 9, 200, 200)],
            "another group, the same scan"
        );
        assert!(view.group(555).is_empty());
        assert_eq!(view.stat(201), Some(stat(201, 200, 200, 200)), "single reads stay live");
        assert_eq!(view.argv(102), Some(vec!["grep".to_owned()]));
        assert_eq!(
            table.group(101),
            [stat(101, 100, 101, 100), stat(102, 100, 101, 100)],
            "the table itself scans each time"
        );
        assert_eq!(
            table.shared_listing().unwrap().group(200).len(),
            2,
            "a new pass scans again"
        );
    }

    #[test]
    fn a_stat_line_gives_the_terminal_s_foreground_group() {
        let root = tempfile::tempdir().unwrap();
        let write = |pid: i32, stat: &[u8]| {
            let dir = root.path().join(pid.to_string());
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("stat"), stat).unwrap();
        };
        let tail = b" 0 0 0 0 1 2 0 0 20 0 1 0 987654 0 0";
        write(
            4242,
            &[&b"4242 (z (s) h) S 4200 4242 4242 34817 4300 4194560"[..], tail].concat(),
        );
        write(7, &[&b"7 (daemon) S 1 7 7 0 -1 0"[..], tail].concat());
        write(8, b"8 (short) S 1 8");
        let table = ProcTable::at(root.path());
        assert_eq!(table.terminal_foreground(4242), Some(4300));
        assert_eq!(table.terminal_foreground(7), None, "no controlling terminal");
        assert_eq!(table.terminal_foreground(8), None);
        assert_eq!(table.terminal_foreground(9), None);
    }

    #[test]
    fn the_live_table_reads_this_test_process() {
        let table = ProcTable::new();
        let pid = i32::try_from(std::process::id()).unwrap();
        let own = table.stat(pid).unwrap();
        assert_eq!(own.pid, pid);
        assert!(!table.argv(pid).unwrap().is_empty());
        assert!(table.group(own.pgrp).contains(&own));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(table.working_directory(pid).as_deref(), cwd.to_str());
        let exe = std::env::current_exe().unwrap();
        assert_eq!(table.executable_path(pid).as_deref(), exe.to_str());
        assert!(table.search_path(pid).is_some(), "the test runner has a PATH");
        assert_eq!(table.executable_path(-1), None);
        assert_eq!(table.search_path(-1), None);
    }

    #[test]
    fn a_fake_tree_answers_the_image_and_the_path() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("7");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("environ"), b"HOME=/h\0PATH=/opt/bin:/usr/bin\0").unwrap();
        std::os::unix::fs::symlink("/usr/bin/codex (deleted)", dir.join("exe")).unwrap();
        let table = ProcTable::at(root.path());
        assert_eq!(table.search_path(7).as_deref(), Some("/opt/bin:/usr/bin"));
        assert_eq!(table.executable_path(7), None, "an upgrade removed the image");
        fs::remove_file(dir.join("exe")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/htop", dir.join("exe")).unwrap();
        assert_eq!(table.executable_path(7).as_deref(), Some("/usr/bin/htop"));
        fs::write(dir.join("environ"), b"HOME=/h\0").unwrap();
        assert_eq!(table.search_path(7), None);
    }
}
