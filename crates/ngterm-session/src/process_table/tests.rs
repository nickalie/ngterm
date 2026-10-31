use super::*;

/// A synthetic `KERN_PROCARGS2` blob (upstream's test helper): host-order argc, the NUL-terminated executable path,
/// `padding` extra NULs, then each argument NUL-terminated.
fn blob(count: i32, path: &str, padding: usize, arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = count.to_ne_bytes().to_vec();
    bytes.extend_from_slice(path.as_bytes());
    bytes.push(0);
    bytes.extend(std::iter::repeat_n(0, padding));
    for arg in arguments {
        bytes.extend_from_slice(arg);
        bytes.push(0);
    }
    bytes
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

#[test]
fn procargs_read_the_argv_past_the_executable_paths_padding() {
    assert_eq!(
        parse_procargs(&blob(2, "/usr/bin/ssh", 3, &[b"ssh", b"gate"])),
        Some(strings(&["ssh", "gate"]))
    );
    assert_eq!(
        parse_procargs(&blob(3, "/usr/bin/ssh", 1, &[b"ssh", b"gate", b"-t ssh inner"])),
        Some(strings(&["ssh", "gate", "-t ssh inner"]))
    );
}

#[test]
fn procargs_refuse_a_truncated_empty_or_implausible_blob() {
    assert_eq!(parse_procargs(&[]), None);
    assert_eq!(
        parse_procargs(&blob(2, "/bin/sh", 0, &[b"sh"])),
        None,
        "fewer arguments than argc"
    );
    assert_eq!(parse_procargs(&[1, 2]), None, "shorter than the argc header");
    assert_eq!(parse_procargs(&blob(0, "/bin/sh", 0, &[])), None);
    assert_eq!(parse_procargs(&blob(-1, "/bin/sh", 1, &[b"sh"])), None);
    assert_eq!(
        parse_procargs(&blob(5000, "/bin/sh", 1, &[b"sh", b"x"])),
        None,
        "beyond the cap"
    );
    let mut unterminated = 1_i32.to_ne_bytes().to_vec();
    unterminated.extend_from_slice(b"/bin/shhhhhh");
    assert_eq!(parse_procargs(&unterminated), None, "no NUL after the path");
}

#[test]
fn procargs_take_an_empty_path_and_ignore_the_environment() {
    assert_eq!(parse_procargs(&blob(1, "", 0, &[b"sh"])), Some(strings(&["sh"])));
    let mut with_env = blob(1, "/bin/sh", 1, &[b"sh"]);
    with_env.extend_from_slice(b"PATH=/bin\0");
    assert_eq!(parse_procargs(&with_env), Some(strings(&["sh"])));
}

#[test]
fn procargs_decode_lossily_and_keep_spaces_and_quotes() {
    assert_eq!(
        parse_procargs(&blob(2, "/usr/bin/grep", 0, &[b"grep", b"caf\xe9"])),
        Some(strings(&["grep", "caf\u{fffd}"]))
    );
    assert_eq!(
        parse_procargs(&blob(3, "/bin/sh", 2, &[b"sh", b"a b", b"it's \"quoted\""])),
        Some(strings(&["sh", "a b", "it's \"quoted\""]))
    );
}

fn member(pid: i32, started: u64, argv: Option<&[&str]>) -> ConsoleMember {
    ConsoleMember {
        pid,
        started,
        argv: argv.map(strings),
    }
}

#[test]
fn a_consoles_foreground_is_its_oldest_program_else_the_shell() {
    let sh = Some(&["sh.exe"][..]);
    let shell = member(100, 10, sh);
    // at its prompt: Git for Windows' launcher runs the shell, an MSYS fork copies it
    let launched = member(101, 11, Some(&["bash.exe", "--login", "-i"]));
    let fork = member(102, 12, Some(&["bash.exe", "--login", "-i"]));
    let at_prompt = [shell.clone(), launched.clone(), fork.clone()];
    assert_eq!(console_foreground(100, &at_prompt), Some(100));
    // `git log`: the program the shell started, not its pager
    let git = member(200, 20, Some(&["git", "log"]));
    let pager = member(201, 21, Some(&["less"]));
    let busy = [shell.clone(), launched, pager.clone(), git, fork];
    assert_eq!(console_foreground(100, &busy), Some(200));
    // a member whose argv cannot be read, or one older than the shell, never stands in
    let hidden = [shell.clone(), member(150, 15, None), member(90, 5, Some(&["old"])), pager];
    assert_eq!(console_foreground(100, &hidden), Some(201));
    assert_eq!(console_foreground(999, &hidden), None, "the shell is gone");
    // equal creation times go to the lower pid
    let tied = [shell, member(301, 30, Some(&["b"])), member(300, 30, Some(&["a"]))];
    assert_eq!(console_foreground(100, &tied), Some(300));
}

#[test]
fn a_command_wrapper_holds_its_consoles_foreground() {
    let wrapper = member(100, 10, Some(&["cmd.exe", "/D", "/S", "/C", "cat"]));
    let cat = member(101, 11, Some(&["cat"]));
    assert_eq!(console_foreground(100, &[wrapper, cat.clone()]), Some(100));
    // a line holding a quoted word, split by the C runtime's rules
    let split = member(
        100,
        10,
        Some(&["cmd.exe", "/D", "/S", "/C", r"C:\Program", r"Files\nodejs\node.exe server.js"]),
    );
    let node = member(101, 11, Some(&[r"C:\Program Files\nodejs\node.exe", "server.js"]));
    assert_eq!(console_foreground(100, &[split, node]), Some(100));
    let interactive = member(100, 10, Some(&["cmd.exe"]));
    assert_eq!(console_foreground(100, &[interactive, cat]), Some(101));
}
