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

/// Whether the nearest Codex among `pid` and its ancestors runs headless; false when none is found or a process
/// cannot be read.
pub(super) fn headless_codex_above(pid: u32) -> bool {
    let mut pid = pid;
    for _ in 0..ANCESTORS {
        let Some(argv) = argv_of(pid) else {
            return false;
        };
        if is_codex(&argv) {
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

    /// A process whose argv says `codex … exec` and a child of it: the walk from the child finds that Codex above
    /// it, as a hook's walk finds the run that fired it; a plain process tree finds none.
    #[cfg(target_os = "linux")]
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
            assert_eq!(headless_codex_above(child), headless, "{arg0} {extra:?}");
            // SAFETY: `kill` has no memory preconditions; the sleep is this test's own
            unsafe { libc::kill(libc::pid_t::try_from(child).unwrap(), libc::SIGKILL) };
            let _ = parent.kill();
            let _ = parent.wait();
        }
    }
}
