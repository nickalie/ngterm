//! Which Codex run fired a hook, told from its process: the nearest `codex` among the hook's ancestors and the
//! subcommand its exact argv names. The rollout's `session_meta` says how a session was created, which a `codex exec
//! resume` of a TUI session leaves as it was, so a headless run is also refused by its own argv.
//!
//! The argv is read whole, never from text a tool joined: Linux's `/proc/<pid>/cmdline` and macOS's
//! `sysctl(KERN_PROCARGS2)`, with the parent from `/proc/<pid>/stat` and `proc_pidinfo(PROC_PIDTBSDINFO)`. Windows,
//! where the Codex adapter runs under Git Bash, reads no process here, so the rollout alone decides there.

/// How many ancestors a hook's process walks through to its Codex: the command substitution and the adapter that run
/// this CLI, the `$SHELL -lc` Codex starts a hook under (fish keeps itself between the two), and some slack.
const ANCESTORS: usize = 6;

/// The Codex options that take a value as the next word, so the subcommand is not found in it.
const VALUED_OPTIONS: [&str; 16] = [
    "-c",
    "--config",
    "-m",
    "--model",
    "-p",
    "--profile",
    "-C",
    "--cd",
    "-s",
    "--sandbox",
    "-a",
    "--ask-for-approval",
    "--add-dir",
    "--enable",
    "--disable",
    "--local-provider",
];

/// The Codex options that take one or more values, the words up to the next option (`--image a.png b.png`).
const MULTI_VALUED_OPTIONS: [&str; 2] = ["-i", "--image"];

/// The subcommands that run Codex without its TUI: `exec` (`e`) and its `resume`, `review`, and the servers.
const HEADLESS: [&str; 5] = ["exec", "e", "review", "app-server", "mcp-server"];

/// Whether the nearest Codex among `pid` and its ancestors runs headless; false when none is found or the chain of
/// parents can't be followed.
pub(super) fn headless_codex_above(pid: u32) -> bool {
    walk(pid, argv_of, parent_of)
}

/// The walk of [`headless_codex_above`], over the given process readers. A process whose argv can't be read or is
/// empty is not a Codex, but the walk goes on through its parent: a process in the middle of an `exec` shows an empty
/// `/proc/<pid>/cmdline` for a moment, and a kernel thread has none.
fn walk(pid: u32, argv_of: impl Fn(u32) -> Option<Vec<String>>, parent_of: impl Fn(u32) -> Option<u32>) -> bool {
    let mut pid = pid;
    for _ in 0..ANCESTORS {
        if let Some(argv) = argv_of(pid)
            && is_codex(&argv)
        {
            return runs_headless(&argv);
        }
        match parent_of(pid) {
            Some(parent) if parent > 1 && parent != pid => pid = parent,
            _ => return false,
        }
    }
    false
}

/// Whether `argv` is a Codex binary's: `codex`, `codex.exe`, or a `codex-*` build of it.
fn is_codex(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    name == "codex" || name == "codex.exe" || name.starts_with("codex-")
}

/// Whether a Codex `argv` names a headless subcommand: its first word that is not an option or an option's value. An
/// image option's values run up to the next option, and a headless subcommand's name among them counts as that
/// subcommand, so where the two cannot be told apart the run takes no mail (it stays in the mailbox).
pub(super) fn runs_headless(argv: &[String]) -> bool {
    let mut words = argv.iter().skip(1).peekable();
    while let Some(word) = words.next() {
        if word == "--" {
            return false; // the prompt follows
        }
        if VALUED_OPTIONS.contains(&word.as_str()) {
            words.next();
            continue;
        }
        if MULTI_VALUED_OPTIONS.contains(&word.as_str()) {
            while let Some(value) = words.next_if(|value| !value.starts_with('-')) {
                if HEADLESS.contains(&value.as_str()) {
                    return true;
                }
            }
            continue;
        }
        if word.starts_with('-') {
            continue;
        }
        return HEADLESS.contains(&word.as_str());
    }
    false
}

#[cfg(target_os = "linux")]
fn argv_of(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv: Vec<String> = raw
        .split(|byte| *byte == 0)
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect();
    // the read ends with the last word's NUL, which leaves an empty word behind
    let argv = match argv.split_last() {
        Some((last, rest)) if last.is_empty() => rest.to_vec(),
        _ => argv,
    };
    (!argv.is_empty()).then_some(argv)
}

#[cfg(target_os = "linux")]
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`, where comm may hold spaces and parentheses: the fields follow its last `)`
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "macos")]
fn argv_of(pid: u32) -> Option<Vec<String>> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let count = libc::c_uint::try_from(mib.len()).ok()?;
    let mut size = 0_usize;
    // SAFETY: `mib` holds `count` names, a null buffer only asks for the size, and `size` is writable.
    let sized = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            count,
            std::ptr::null_mut(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if sized != 0 || size == 0 {
        return None;
    }
    let mut buffer = vec![0_u8; size];
    // SAFETY: `buffer` holds `size` writable bytes, which `size` tells the kernel.
    let read = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            count,
            buffer.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return None;
    }
    buffer.truncate(size);
    parse_procargs(&buffer)
}

/// The argv in a `KERN_PROCARGS2` blob: a host-order `i32` argc, the NUL-terminated executable path, NUL padding, then
/// `argc` NUL-terminated words (as `ngterm-session`'s process table reads it).
#[cfg(any(target_os = "macos", test))]
fn parse_procargs(bytes: &[u8]) -> Option<Vec<String>> {
    let (head, rest) = bytes.split_first_chunk::<4>()?;
    let count = usize::try_from(i32::from_ne_bytes(*head))
        .ok()
        .filter(|count| (1..=4096).contains(count))?;
    let path_end = rest.iter().position(|byte| *byte == 0)?;
    let start = rest[path_end..].iter().position(|byte| *byte != 0)? + path_end;
    let mut words = rest[start..].split(|byte| *byte == 0);
    (0..count)
        .map(|_| words.next().map(|word| String::from_utf8_lossy(word).into_owned()))
        .collect()
}

#[cfg(target_os = "macos")]
fn parent_of(pid: u32) -> Option<u32> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: `info` has room for the `size` bytes of a `proc_bsdinfo`, which PROC_PIDTBSDINFO writes.
    let read = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if read != size {
        return None;
    }
    // SAFETY: the call filled the whole struct, and it was zeroed before.
    Some(unsafe { info.assume_init() }.pbi_ppid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn argv_of(_pid: u32) -> Option<Vec<String>> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn parent_of(_pid: u32) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    // The flake CI hit (2026-11-07, `the_walk_finds_the_codex_a_hook_runs_under`): the hook's own process read while it
    // was in the middle of an `exec`, so its argv came back empty and the walk stopped there instead of reaching the
    // Codex above it.
    #[test]
    fn a_process_with_no_argv_is_walked_through_to_its_parent() {
        let parents = |pid| match pid {
            30 => Some(20),
            20 => Some(10),
            _ => None,
        };
        let mid_exec = |pid| match pid {
            // 30, mid-exec, has no argv
            20 => Some(argv(&["codex", "exec", "task"])),
            _ => None,
        };
        assert!(walk(30, mid_exec, parents));
        let tui = |pid| match pid {
            // 30, mid-exec, has no argv
            20 => Some(argv(&["codex", "--no-daemon"])),
            _ => None,
        };
        assert!(!walk(30, tui, parents));
        // no Codex anywhere, or a chain that can't be followed, is not headless
        assert!(!walk(30, |_| None, parents));
        assert!(!walk(30, mid_exec, |_| None));
    }

    /// The walk looks at the hook's process and its five nearest ancestors: a Codex further up is not the run that
    /// fired the hook, so a chain that long with no Codex in it is not headless.
    #[test]
    fn a_codex_past_the_walks_reach_is_not_found() {
        // 100's parent is 99, 99's is 98, and so on: a chain longer than the walk
        let parents = |pid: u32| (pid > 2).then(|| pid - 1);
        let headless_at = |codex: u32| move |pid| (pid == codex).then(|| argv(&["codex", "exec", "task"]));
        assert!(walk(100, headless_at(95), parents), "the sixth process walked");
        assert!(!walk(100, headless_at(94), parents), "the seventh, past the walk");
    }

    /// Neither Linux nor macOS: no process is read, so no Codex is found above any process, this one included, and
    /// the rollout alone decides.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn no_process_is_read_on_other_systems() {
        let me = std::process::id();
        assert_eq!(argv_of(me), None);
        assert_eq!(parent_of(me), None);
        assert!(!headless_codex_above(me));
    }

    #[test]
    fn the_subcommand_is_the_first_word_past_the_options() {
        for headless in [
            &["codex", "exec", "task"][..],
            &["/usr/local/bin/codex", "e", "task"],
            &["codex", "exec", "resume", "--last", "task"],
            &[
                "codex",
                "-c",
                "model=\"x y\"",
                "-C",
                "/Users/me/My Project",
                "exec",
                "task",
            ],
            &["codex", "--full-auto", "--model", "o9", "review"],
            &["codex-x86_64-unknown-linux-musl", "app-server"],
            &["codex.exe", "mcp-server"],
            &[
                "codex",
                "--image",
                "before.png",
                "after.png",
                "-C",
                ".",
                "exec",
                "resume",
                "id",
            ],
            &["codex", "-i", "a.png", "exec", "task"],
            &["codex", "-i", "a.png", "b.png", "review"],
        ] {
            assert!(runs_headless(&argv(headless)), "{headless:?}");
        }
        for interactive in [
            &["codex"][..],
            &["codex", "review the diff"],
            &["codex", "--", "exec"],
            &["codex", "-m", "exec"],
            &["codex", "resume", "--last"],
            &["codex", "fork"],
            &["codex", "--no-daemon", "fix the build"],
            &["codex", "--image", "a.png", "b.png", "--", "exec"],
            &["codex", "--image=a.png", "fix it"],
        ] {
            assert!(!runs_headless(&argv(interactive)), "{interactive:?}");
        }
    }

    #[test]
    fn only_a_codex_binary_counts() {
        assert!(is_codex(&argv(&["codex"])));
        assert!(is_codex(&argv(&["/opt/codex/bin/codex-aarch64-apple-darwin"])));
        assert!(is_codex(&argv(&["C:\\tools\\codex.exe"])));
        assert!(!is_codex(&argv(&["fish"])));
        assert!(!is_codex(&argv(&["codexy"])));
        assert!(!is_codex(&[]));
    }

    #[test]
    fn a_procargs_blob_yields_its_argv() {
        let mut blob = 3_i32.to_ne_bytes().to_vec();
        blob.extend_from_slice(b"/usr/local/bin/codex\0\0\0codex\0exec\0my task\0PATH=/bin\0");
        assert_eq!(parse_procargs(&blob), Some(argv(&["codex", "exec", "my task"])));
        assert_eq!(parse_procargs(&blob[..2]), None);
        let mut short = 4_i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/codex\0codex\0exec");
        assert_eq!(parse_procargs(&short), None);
    }

    #[test]
    fn an_invalid_procargs_count_or_missing_argv_is_rejected() {
        for count in [-1_i32, 0, 4097] {
            let mut blob = count.to_ne_bytes().to_vec();
            blob.extend_from_slice(b"/bin/codex\0codex\0");
            assert_eq!(parse_procargs(&blob), None, "argc {count}");
        }
        for missing in [&b"/bin/codex"[..], &b"/bin/codex\0\0\0"[..]] {
            let mut blob = 1_i32.to_ne_bytes().to_vec();
            blob.extend_from_slice(missing);
            assert_eq!(parse_procargs(&blob), None, "{missing:?}");
        }
    }

    /// A process whose argv says `codex … exec` and a child of it: the walk from the child finds that Codex above
    /// it, as a hook's walk finds the run that fired it; a plain process tree finds none.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_walk_finds_the_codex_a_hook_runs_under() {
        use std::io::{BufRead as _, BufReader};
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};

        let spawn = |arg0: &str, script: &str, extra: &[&str]| {
            let mut parent = Command::new("sh")
                .arg0(arg0)
                .arg("-c")
                .arg(script)
                .args(extra)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut line = String::new();
            BufReader::new(parent.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            (parent, line.trim().parse::<u32>().unwrap())
        };
        let script = "sleep 30 & echo $!; wait";
        for (arg0, extra, headless) in [
            ("codex", &["exec"][..], true),
            ("codex", &["--no-daemon"], false),
            ("sh", &[], false),
        ] {
            let (mut parent, child) = spawn(arg0, script, extra);
            let actual = headless_codex_above(child);
            // SAFETY: `kill` has no memory preconditions; the sleep is this test's own
            unsafe { libc::kill(libc::pid_t::try_from(child).unwrap(), libc::SIGKILL) };
            let _ = parent.kill();
            let _ = parent.wait();
            assert_eq!(actual, headless, "{arg0} {extra:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reads_a_childs_exact_argv_and_parent() {
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};

        let probe = "agent::mail::codex_process::tests::macos_argv_probe";
        for (subcommand, headless) in [("exec", true), ("resume", false)] {
            let arguments = [subcommand, "--ignored", "--exact", probe, "", "a b", "a\"b", "日本語"];
            let mut child = Command::new(std::env::current_exe().unwrap())
                .arg0("/test tools/codex-macos")
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let actual_argv = argv_of(child.id());
            let actual_parent = parent_of(child.id());
            let actual_headless = headless_codex_above(child.id());
            drop(child.stdin.take());
            let status = child.wait().unwrap();

            let mut expected = argv(&["/test tools/codex-macos"]);
            expected.extend(argv(&arguments));
            assert!(status.success(), "{status}");
            assert_eq!(actual_argv, Some(expected));
            assert_eq!(actual_parent, Some(std::process::id()));
            assert_eq!(actual_headless, headless);
            assert_eq!(argv_of(child.id()), None, "a reaped child has no argv");
            assert_eq!(parent_of(child.id()), None, "a reaped child has no parent");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "the process reader's child, released by closing its stdin"]
    fn macos_argv_probe() {
        use std::io::Read as _;
        std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_rejects_missing_and_out_of_range_processes() {
        for pid in [i32::MAX.cast_unsigned(), u32::MAX] {
            assert_eq!(argv_of(pid), None, "{pid}");
            assert_eq!(parent_of(pid), None, "{pid}");
            assert!(!headless_codex_above(pid), "{pid}");
        }
        // SAFETY: `getppid` has no preconditions and reads only this process's parent id.
        let parent = unsafe { libc::getppid() };
        assert_eq!(parent_of(std::process::id()), Some(parent.cast_unsigned()));
    }
}
