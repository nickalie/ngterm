//! The script a wrapped pane's daemon runs a command with (upstream's `ZmxReplayScript.swift`): the command, then the
//! shell integration re-armed and the login shell exec'd, so the pane ends at a prompt whatever the command did, a
//! Ctrl+C included. The script runs in an interactive shell (the pane's job control and the user's startup files),
//! which drops the rest of a plain `;` list once its foreground job is interrupted, so each shell's form keeps the
//! re-arm and the `exec` out of the command's list.
//!
//! zsh and bash share the POSIX words, where every builtin is quoted, so a user's alias of `builtin`, `export`, `exec`
//! or `trap` cannot take it over. zsh runs the re-arm and the `exec` in an `always` block (upstream's form). bash has no
//! `always`: an interrupted job ends the `-c` string's list, and inside a loop or a list the whole string, so the
//! re-arm and the `exec` are set as an `EXIT` trap first, and follow the command again on a line of their own for a
//! command that returns (the `exec` leaves the trap behind unrun). bash runs an `EXIT` trap on every exit, an explicit
//! `exit` and a hangup's included, where zsh's `always` block is skipped, so the trap execs the final shell only when
//! the string was dropped for an interrupt: the last pipeline then reads 130 (128 + SIGINT) in `PIPESTATUS`, and `$?`
//! reads 1, the status bash gives a dropped string. Neither alone tells: a command that returns 130 itself leaves the
//! same `PIPESTATUS` to an `exit` right after it (the `exit` builtin runs the trap before its own status is recorded),
//! which puts its own status in `$?`, and a hangup keeps the previous command's status in both. A hangup (the session
//! server ending the program) or an `exit` thus ends the session, as under zsh and fish, except an `exit 1` right after
//! a pipeline whose first command returned 130 by itself, which bash leaves nothing to tell from an interrupt. A `HUP`
//! trap is no way to tell either: bash defers it until the foreground job ends, and forwards no hangup to the job
//! meanwhile.
//!
//! fish has no aliases (its `alias` defines a function, which `builtin` bypasses, and `exec` is a reserved word no
//! function can take), so its form is `builtin set -gx` and a plain `exec`, with fish's own quoting. The command runs
//! through `builtin eval`, which confines an interruption to it, under `builtin status job-control full`, which also
//! gives it a process group of its own in the pane's foreground (upstream's form). Its re-arm recomputes
//! `XDG_DATA_DIRS` from the value the command left ([`Rearm::FishDataDirs`], upstream's `begin … end` block with
//! `__ngterm_` names), so the final fish finds its integration and keeps what the command set.
//!
//! PowerShell, a Rust-only extension, has no `exec`: its replay runs in the shell `-NoExit` keeps
//! ([`LoginShell::script_flags`]), so the script re-arms the integration's variable and loads the integration first,
//! then runs the command (`& 'program' 'argument'`, or a durable line as a script block). Loading it first keeps the
//! prompt integrated however the command ends: a Ctrl+C stops the rest of a `-Command` script, not only the command.
//! Only language syntax and .NET calls, so no alias or function of the user's takes it over. The command is
//! [`powershell_call`]'s (`crate::shell_dialect`, which a rerun restore types into a PowerShell pane too): words
//! single-quoted ([`powershell_quoted`]), and an argv that PowerShell's argument passings would not all hand on alike
//! (an empty argument, whitespace, a double quote) run in a scope of its own under the legacy passing, each argument
//! quoted for the C runtime's split, which hands it on whole on Windows PowerShell 5.1 and every PowerShell 7, on every
//! OS, the user's preference left as it is for later commands. Exact for programs that split their command line as the
//! C runtime does; `cmd.exe`, batch files and `msiexec` parse theirs their own way.
use super::shell_integration::{DEFAULT_XDG_DATA_DIRS, LoginShell, PWSH_LOAD, Rearm};
use crate::command_restore::shell_quoted_line;
use crate::shell_dialect::powershell_call;
pub use crate::shell_dialect::{fish_quoted, powershell_quoted};

/// The interactive login shell a replay ends in, and what it re-arms before the `exec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalShell<'a> {
    /// The login shell's path.
    pub path: &'a str,
    pub kind: LoginShell,
    pub rearm: &'a [Rearm],
}

/// The script for a captured `argv`, quoted word by word.
#[must_use]
pub fn render_argv<S: AsRef<str>>(argv: &[S], shell: &FinalShell<'_>) -> String {
    let words = |quote: fn(&str) -> String| {
        argv.iter()
            .map(|word| quote(word.as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let statement = if shell.kind.is_fish() {
        words(fish_quoted)
    } else if shell.kind.is_powershell() {
        powershell_call(argv)
    } else {
        shell_quoted_line(argv)
    };
    render(&statement, shell)
}

/// The script for a durable creation command, a shell line kept with its operators and word parsing (`eval`; in
/// PowerShell a script block made of it, dot-sourced).
#[must_use]
pub fn render_command_line(command_line: &str, shell: &FinalShell<'_>) -> String {
    let statement = if shell.kind.is_fish() {
        // the fish form evaluates its statement itself
        command_line.to_owned()
    } else if shell.kind.is_powershell() {
        format!(". ([scriptblock]::Create({}))", powershell_quoted(command_line))
    } else {
        format!("'builtin' 'eval' -- {}", quoted(command_line))
    };
    render(&statement, shell)
}

fn render(statement: &str, shell: &FinalShell<'_>) -> String {
    match shell.kind {
        LoginShell::Pwsh => render_powershell(statement, shell),
        LoginShell::Fish => render_fish(statement, shell),
        LoginShell::Zsh => format!("{{ {statement} ; }} always {{ {} ; }}", posix_final_shell(shell)),
        LoginShell::Bash => {
            let final_shell = posix_final_shell(shell);
            let on_interrupt =
                format!("case \"${{PIPESTATUS[0]}}:$?\" in {INTERRUPTED}:{DROPPED}) {final_shell} ;; esac");
            format!(
                "'builtin' 'trap' -- {} EXIT\n{statement}\n{final_shell}",
                quoted(&on_interrupt)
            )
        }
    }
}

/// The status bash records for a pipeline SIGINT ended (128 + SIGINT, which is 2 on every Unix), which its `EXIT`
/// trap reads in `PIPESTATUS` when an interrupt dropped the script.
const INTERRUPTED: u8 = 130;

/// The status bash exits with when an interrupt dropped the rest of its `-c` string, which its `EXIT` trap reads in
/// `$?`.
const DROPPED: u8 = 1;

/// The POSIX re-arm and `exec` of the final shell, joined with ` ; `.
fn posix_final_shell(shell: &FinalShell<'_>) -> String {
    let mut steps: Vec<String> = shell
        .rearm
        .iter()
        .filter_map(|step| match step {
            Rearm::Export(key, value) => Some(format!("'builtin' 'export' {key}={}", quoted(value))),
            Rearm::Unset(key) => Some(format!("'builtin' 'unset' {key}")),
            // only fish's re-arm carries it
            Rearm::FishDataDirs(_) => None,
        })
        .collect();
    steps.push(format!(
        "'builtin' 'exec' -- {} {}",
        quoted(shell.path),
        shell.kind.interactive_flags().join(" ")
    ));
    steps.join(" ; ")
}

/// fish's replay: job control on, the statement evaluated, the integration re-armed and the final shell exec'd, joined
/// with `; `.
fn render_fish(statement: &str, shell: &FinalShell<'_>) -> String {
    let mut steps = vec![
        "builtin status job-control full".to_owned(),
        format!("builtin eval {}", fish_quoted(statement)),
    ];
    steps.extend(shell.rearm.iter().map(|step| match step {
        Rearm::Export(key, value) => format!("builtin set -gx {key} {}", fish_quoted(value)),
        Rearm::Unset(key) => format!("builtin set -e {key}"),
        Rearm::FishDataDirs(root) => fish_data_dirs(root),
    }));
    steps.push(format!(
        "exec {} {}",
        fish_quoted(shell.path),
        shell.kind.interactive_flags().join(" ")
    ));
    steps.join("; ")
}

/// The fish block that exports `XDG_DATA_DIRS` as `root`, then the entries of the value the command left (the
/// specification's default when it is unset or empty), each empty entry and each copy of `root` left out: upstream's
/// `renderFish` re-arm, with its variables named `__ngterm_`, local to the block, and `string` taken as the builtin.
fn fish_data_dirs(root: &str) -> String {
    let root = fish_quoted(root);
    [
        "begin".to_owned(),
        "builtin set --local __ngterm_xdg_source".to_owned(),
        "if builtin set -q XDG_DATA_DIRS; and builtin test -n \"$XDG_DATA_DIRS\"".to_owned(),
        "builtin set __ngterm_xdg_source \"$XDG_DATA_DIRS\"".to_owned(),
        "else".to_owned(),
        format!("builtin set __ngterm_xdg_source {DEFAULT_XDG_DATA_DIRS}"),
        "end".to_owned(),
        "builtin set --local __ngterm_xdg_user_dirs".to_owned(),
        "builtin set --local __ngterm_xdg_data_dir".to_owned(),
        "for __ngterm_xdg_data_dir in (builtin string split : -- \"$__ngterm_xdg_source\")".to_owned(),
        format!("if builtin test -n \"$__ngterm_xdg_data_dir\"; and builtin test \"$__ngterm_xdg_data_dir\" != {root}"),
        "builtin set --append __ngterm_xdg_user_dirs \"$__ngterm_xdg_data_dir\"".to_owned(),
        "end".to_owned(),
        "end".to_owned(),
        format!("builtin set --global --export --path XDG_DATA_DIRS {root} $__ngterm_xdg_user_dirs"),
        "end".to_owned(),
    ]
    .join("; ")
}

/// PowerShell's replay: the re-armed variables, the integration loaded, then the command, in the shell that stays.
fn render_powershell(statement: &str, shell: &FinalShell<'_>) -> String {
    let mut steps: Vec<String> = shell
        .rearm
        .iter()
        .filter_map(|step| match step {
            Rearm::Export(key, value) => Some(format!("$env:{key} = {}", powershell_quoted(value))),
            Rearm::Unset(key) => Some(format!("$env:{key} = $null")),
            // only fish's re-arm carries it
            Rearm::FishDataDirs(_) => None,
        })
        .collect();
    steps.push(PWSH_LOAD.to_owned());
    steps.push(statement.to_owned());
    steps.join(" ; ")
}

fn quoted(value: &str) -> String {
    shell_quoted_line(&[value])
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;

    fn zsh_rearm(integration_dir: &str, inherited_zdotdir: Option<&str>) -> Vec<Rearm> {
        vec![
            Rearm::Export("ZDOTDIR", integration_dir.to_owned()),
            inherited_zdotdir.map_or(Rearm::Unset("GHOSTTY_ZSH_ZDOTDIR"), |dir| {
                Rearm::Export("GHOSTTY_ZSH_ZDOTDIR", dir.to_owned())
            }),
        ]
    }

    /// fish's re-arm for the integration under `root`, as `shell_integration::rearm` reads it off a wrap.
    fn fish_rearm(root: &str) -> Vec<Rearm> {
        vec![
            Rearm::Export("GHOSTTY_SHELL_INTEGRATION_XDG_DIR", root.to_owned()),
            Rearm::FishDataDirs(root.to_owned()),
        ]
    }

    fn zsh<'a>(rearm: &'a [Rearm], path: &'a str) -> FinalShell<'a> {
        FinalShell {
            path,
            kind: LoginShell::Zsh,
            rearm,
        }
    }

    #[test]
    fn a_custom_zdotdir_is_rearmed_before_the_final_shell() {
        let rearm = zsh_rearm("/Applications/ngterm resources/zsh", Some("/Users/test/.config/zsh"));
        let script = render_argv(&["printf", "a b"], &zsh(&rearm, "/bin/zsh"));
        assert_eq!(
            script,
            "{ 'printf' 'a b' ; } always { 'builtin' 'export' ZDOTDIR='/Applications/ngterm resources/zsh' ; \
             'builtin' 'export' GHOSTTY_ZSH_ZDOTDIR='/Users/test/.config/zsh' ; 'builtin' 'exec' -- '/bin/zsh' -il ; }"
        );
    }

    #[test]
    fn a_missing_original_zdotdir_is_unset_before_the_final_shell() {
        let rearm = zsh_rearm("/bundle/zsh", None);
        assert_eq!(
            render_argv(&["top"], &zsh(&rearm, "/bin/zsh")),
            "{ 'top' ; } always { 'builtin' 'export' ZDOTDIR='/bundle/zsh' ; 'builtin' 'unset' GHOSTTY_ZSH_ZDOTDIR ; \
             'builtin' 'exec' -- '/bin/zsh' -il ; }"
        );
    }

    #[test]
    fn a_command_line_is_evaluated_as_one_quoted_word() {
        let rearm = zsh_rearm("/z", None);
        assert_eq!(
            render_command_line("a && b 'c'", &zsh(&rearm, "/bin/zsh")),
            r"{ 'builtin' 'eval' -- 'a && b '\''c'\''' ; } always { 'builtin' 'export' ZDOTDIR='/z' ; 'builtin' 'unset' GHOSTTY_ZSH_ZDOTDIR ; 'builtin' 'exec' -- '/bin/zsh' -il ; }"
        );
    }

    #[test]
    fn bash_rearms_its_posix_mode_injection_and_execs_bash_in_posix_mode_from_an_interrupt_only_exit_trap_as_well() {
        let rearm = [
            Rearm::Export("ENV", "/res/bash/ghostty.bash".to_owned()),
            Rearm::Export("GHOSTTY_BASH_INJECT", "1".to_owned()),
            Rearm::Unset("GHOSTTY_BASH_ENV"),
        ];
        let shell = FinalShell {
            path: "/bin/bash",
            kind: LoginShell::Bash,
            rearm: &rearm,
        };
        let final_shell = "'builtin' 'export' ENV='/res/bash/ghostty.bash' ; 'builtin' 'export' \
                           GHOSTTY_BASH_INJECT='1' ; 'builtin' 'unset' GHOSTTY_BASH_ENV ; 'builtin' 'exec' -- \
                           '/bin/bash' --posix -il";
        let trap = r#"'builtin' 'trap' -- 'case "${PIPESTATUS[0]}:$?" in 130:1) '\''builtin'\'' '\''export'\'' ENV='\''/res/bash/ghostty.bash'\'' ; '\''builtin'\'' '\''export'\'' GHOSTTY_BASH_INJECT='\''1'\'' ; '\''builtin'\'' '\''unset'\'' GHOSTTY_BASH_ENV ; '\''builtin'\'' '\''exec'\'' -- '\''/bin/bash'\'' --posix -il ;; esac' EXIT"#;
        assert_eq!(render_argv(&["top"], &shell), format!("{trap}\n'top'\n{final_shell}"));
        assert_eq!(
            render_command_line("a; b", &shell),
            format!("{trap}\n'builtin' 'eval' -- 'a; b'\n{final_shell}")
        );
    }

    #[test]
    fn fish_evaluates_the_command_under_job_control_then_rearms_and_execs_fish_in_its_own_quoting() {
        let rearm = fish_rearm("/res/it's");
        let shell = FinalShell {
            path: "/usr/bin/fish",
            kind: LoginShell::Fish,
            rearm: &rearm,
        };
        // upstream's re-arm block, its variables named `__ngterm_`
        let tail = concat!(
            r"builtin set -gx GHOSTTY_SHELL_INTEGRATION_XDG_DIR '/res/it\'s'; ",
            "begin; builtin set --local __ngterm_xdg_source; ",
            r#"if builtin set -q XDG_DATA_DIRS; and builtin test -n "$XDG_DATA_DIRS"; "#,
            r#"builtin set __ngterm_xdg_source "$XDG_DATA_DIRS"; "#,
            "else; builtin set __ngterm_xdg_source /usr/local/share:/usr/share; end; ",
            "builtin set --local __ngterm_xdg_user_dirs; builtin set --local __ngterm_xdg_data_dir; ",
            r#"for __ngterm_xdg_data_dir in (builtin string split : -- "$__ngterm_xdg_source"); "#,
            r#"if builtin test -n "$__ngterm_xdg_data_dir"; and builtin test "$__ngterm_xdg_data_dir" != '/res/it\'s'; "#,
            r#"builtin set --append __ngterm_xdg_user_dirs "$__ngterm_xdg_data_dir"; end; end; "#,
            r"builtin set --global --export --path XDG_DATA_DIRS '/res/it\'s' $__ngterm_xdg_user_dirs; end; ",
            "exec '/usr/bin/fish' -il",
        );
        // the captured words are quoted once as words, then again as the one word `eval` takes
        assert_eq!(
            render_argv(&["printf", r"a\b", "it's"], &shell),
            format!(r"builtin status job-control full; builtin eval '\'printf\' \'a\\\\b\' \'it\\\'s\''; {tail}")
        );
        assert_eq!(
            render_command_line("a && b 'c'", &shell),
            format!(r"builtin status job-control full; builtin eval 'a && b \'c\''; {tail}")
        );
        let unset = [Rearm::Unset("X")];
        assert!(
            render_argv(&["top"], &FinalShell { rearm: &unset, ..shell })
                .ends_with("; builtin set -e X; exec '/usr/bin/fish' -il")
        );
        // upstream's argv case: a dollar, a semicolon and a glob, spaces, a quote and an empty word
        let argv = ["/tmp/tool $name;*", "two words", "can't", ""];
        let statement = r"'/tmp/tool $name;*' 'two words' 'can\'t' ''";
        assert_eq!(
            render_argv(&argv, &shell),
            format!(
                "builtin status job-control full; builtin eval {}; {tail}",
                fish_quoted(statement)
            )
        );
    }

    #[test]
    fn fish_quoting_covers_required_and_hostile_arguments() {
        let cases = [
            (r"trailing\", r"'trailing\\'"),
            (r"two\\slashes", r"'two\\\\slashes'"),
            ("can't", r"'can\'t'"),
            ("", "''"),
            (r"\'", r"'\\\''"),
            (r"many\\\slashes", r"'many\\\\\\slashes'"),
            ("two words", "'two words'"),
            (
                "\u{e9}\u{43b}\u{435}\u{43d}\u{430}",
                "'\u{e9}\u{43b}\u{435}\u{43d}\u{430}'",
            ),
            ("$literal", "'$literal'"),
            ("(parentheses)", "'(parentheses)'"),
            ("*.swift", "'*.swift'"),
            ("semi;colon", "'semi;colon'"),
            (">redirect", "'>redirect'"),
            ("--option", "'--option'"),
            ("line one\nline two", "'line one\nline two'"),
        ];
        let rearm = [Rearm::Export("GHOSTTY_SHELL_INTEGRATION_XDG_DIR", "/root".to_owned())];
        let shell = FinalShell {
            path: "/bin/fish",
            kind: LoginShell::Fish,
            rearm: &rearm,
        };
        for (argument, expected) in cases {
            assert_eq!(fish_quoted(argument), expected, "{argument:?}");
            let prefix = format!(
                "builtin status job-control full; builtin eval {}; ",
                fish_quoted(expected)
            );
            assert!(render_argv(&[argument], &shell).starts_with(&prefix), "{argument:?}");
        }
    }

    fn executable(path: &Path, contents: &str) {
        ngterm_host::fakes::write_executable(path, contents).unwrap();
    }

    /// The POSIX login shells this host has, each with the form it is replayed in: bash always (every Unix CI host),
    /// zsh where it is installed.
    fn posix_shells() -> Vec<(LoginShell, &'static str)> {
        [
            (LoginShell::Bash, ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]),
            (LoginShell::Zsh, ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]),
        ]
        .into_iter()
        .filter_map(|(kind, paths)| {
            paths
                .into_iter()
                .find(|path| Path::new(path).exists())
                .map(|path| (kind, path))
        })
        .collect()
    }

    /// Runs `script` under the shell at `path` without its startup files.
    fn run(path: &str, script: &str) -> bool {
        let flag = if path.ends_with("zsh") { "-fc" } else { "-c" };
        Command::new(path).arg(flag).arg(script).status().unwrap().success()
    }

    /// The `record-argv` helper in `dir`: it writes its arguments after the first, each ended by a NUL, into the first.
    fn recorder(dir: &Path) -> std::path::PathBuf {
        let record = dir.join("record-argv");
        executable(
            &record,
            "#!/bin/sh\nresult=$1\nshift\nprintf '%s\\0' \"$@\" > \"$result\"\n",
        );
        record
    }

    /// The NUL-ended values the file at `path` holds.
    fn recorded(path: &Path) -> Vec<String> {
        let contents = fs::read_to_string(path).unwrap();
        let values = contents.strip_suffix('\0').unwrap_or_else(|| panic!("{contents:?}"));
        values.split('\0').map(str::to_owned).collect()
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its script with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn captured_arguments_survive_the_shell_round_trip() {
        let expected = [
            "two words",
            "$HOME",
            "single'quote",
            "semi;colon",
            "",
            "line one\nline two",
        ];
        for (kind, path) in posix_shells() {
            let dir = tempfile::tempdir().unwrap();
            let result = dir.path().join("argv");
            let record = recorder(dir.path());
            let mut argv = vec![record.display().to_string(), result.display().to_string()];
            argv.extend(expected.iter().map(|arg| (*arg).to_owned()));
            let rearm = zsh_rearm("/bundle/zsh", None);
            let shell = FinalShell {
                path: "/usr/bin/true",
                kind,
                rearm: &rearm,
            };
            assert!(run(path, &render_argv(&argv, &shell)), "{path}");
            assert_eq!(recorded(&result), expected, "{path}");
        }
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its script with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn a_durable_shell_line_keeps_its_operators_and_word_parsing() {
        for (kind, path) in posix_shells() {
            let dir = tempfile::tempdir().unwrap();
            let result = dir.path().join("shell-line");
            let output = shell_quoted_line(&[result.display().to_string()]);
            let line = format!("printf '%s' 'two words' > {output} && printf ' and more' >> {output}");
            let rearm = zsh_rearm("/bundle/zsh", None);
            let shell = FinalShell {
                path: "/usr/bin/true",
                kind,
                rearm: &rearm,
            };
            assert!(run(path, &render_command_line(&line, &shell)), "{path}");
            assert_eq!(fs::read_to_string(&result).unwrap(), "two words and more", "{path}");
        }
    }

    /// A command that fails or is missing still ends at the final shell, which records the re-armed variables and its
    /// flags once.
    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its script with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn a_failed_command_still_execs_the_rearmed_final_shell_once() {
        for (kind, path) in posix_shells() {
            let dir = tempfile::tempdir().unwrap();
            let result = dir.path().join("final");
            let printer = dir.path().join("final-shell");
            executable(
                &printer,
                &format!(
                    "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$ZDOTDIR\" \"${{GHOSTTY_ZSH_ZDOTDIR-unset}}\" \"$*\" >> {}\n",
                    shell_quoted_line(&[result.display().to_string()])
                ),
            );
            let rearm = zsh_rearm("/bundle/integration", Some("/user/zsh"));
            let printer_path = printer.display().to_string();
            let shell = FinalShell {
                path: &printer_path,
                kind,
                rearm: &rearm,
            };
            let flags = kind.interactive_flags().join(" ");
            for script in [
                render_argv(&["/usr/bin/false"], &shell),
                render_argv(&["/definitely/missing/ngterm-payload"], &shell),
                render_command_line("false && true", &shell),
            ] {
                assert!(run(path, &script), "{path}: {script}");
                assert_eq!(
                    fs::read_to_string(&result).unwrap(),
                    format!("/bundle/integration|/user/zsh|{flags}\n"),
                    "{path}: {script}"
                );
                fs::remove_file(&result).unwrap();
            }
        }
    }

    /// fish, when this host has one (the `linux` CI job installs it for this test): `None` otherwise.
    fn fish() -> Option<&'static str> {
        [
            "/usr/bin/fish",
            "/bin/fish",
            "/usr/local/bin/fish",
            "/opt/homebrew/bin/fish",
        ]
        .into_iter()
        .find(|path| Path::new(path).exists())
    }

    /// Runs `script` under `fish --no-config` in a throwaway home, so no user configuration is read or written, with
    /// `XDG_DATA_DIRS` as `data_dirs` gives it (`None` unsets it).
    fn run_fish(fish: &str, script: &str, home: &Path, data_dirs: Option<&str>) -> std::process::Output {
        let mut command = Command::new(fish);
        match data_dirs {
            Some(value) => command.env("XDG_DATA_DIRS", value),
            None => command.env_remove("XDG_DATA_DIRS"),
        };
        command
            .args(["--no-config", "-c", script])
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    #[test]
    #[cfg_attr(windows, ignore = "runs the replay under fish, which Windows has not")]
    fn a_fish_replay_round_trips_arguments_and_rearms_before_the_exec() {
        let Some(fish) = fish() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let result = dir.path().join("argv");
        let record = recorder(dir.path());
        // upstream's hostile arguments
        let expected = [
            r"trailing\",
            r"two\\slashes",
            "can't",
            r"\'",
            r"many\\\slashes",
            "two words",
            "\u{e9}\u{43b}\u{435}\u{43d}\u{430}",
            "$literal",
            "(parentheses)",
            "*.swift",
            "semi;colon",
            ">redirect",
            "--option",
            "",
            "line one\nline two",
        ];
        let mut argv = vec![record.display().to_string(), result.display().to_string()];
        argv.extend(expected.iter().map(|arg| (*arg).to_owned()));
        // the final "shell" prints the re-armed variables, standing in for the interactive fish
        let printer = dir.path().join("print-env");
        executable(
            &printer,
            "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$XDG_DATA_DIRS\" \"$GHOSTTY_SHELL_INTEGRATION_XDG_DIR\" \"$*\"\n",
        );
        let rearm = fish_rearm("/res/it's");
        let printer_path = printer.display().to_string();
        let shell = FinalShell {
            path: &printer_path,
            kind: LoginShell::Fish,
            rearm: &rearm,
        };
        let output = run_fish(fish, &render_argv(&argv, &shell), dir.path(), Some("/usr/share"));
        assert!(output.status.success(), "{output:?}");
        assert_eq!(recorded(&result), expected);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "/res/it's:/usr/share|/res/it's|-il\n"
        );
        // a durable line keeps its operators and fish's word parsing
        let line_result = dir.path().join("line");
        let line = format!(
            "printf '%s' 'two words' > {0} && printf ' and more' >> {0}",
            fish_quoted(&line_result.display().to_string())
        );
        let output = run_fish(fish, &render_command_line(&line, &shell), dir.path(), None);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(fs::read_to_string(&line_result).unwrap(), "two words and more");
        // a failed or missing command still ends at the final shell, on the default data directories when the session
        // has none (upstream's fishReplayReachesTheFinalShellAfterFailedPayloads)
        for script in [
            render_argv(&["/usr/bin/false"], &shell),
            render_argv(&["/definitely/missing/ngterm-payload"], &shell),
            render_command_line("false; and true", &shell),
            render_command_line("'/definitely/missing/ngterm-payload'", &shell),
        ] {
            let output = run_fish(fish, &script, dir.path(), None);
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "/res/it's:/usr/local/share:/usr/share|/res/it's|-il\n",
                "{script}"
            );
        }
    }

    /// upstream's `fishReplayRearmsXDGDataDirectoriesFromTheLivePayloadState`: the final fish's `XDG_DATA_DIRS` is the
    /// root, then what the command left there (the default when it emptied or unset it), copies of the root dropped.
    #[test]
    #[cfg_attr(windows, ignore = "runs the replay under fish, which Windows has not")]
    fn a_fish_replay_rearms_the_data_dirs_from_what_the_command_left() {
        let Some(fish) = fish() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let printer = dir.path().join("print-env");
        executable(
            &printer,
            "#!/bin/sh\nprintf '%s|%s\\n' \"$XDG_DATA_DIRS\" \"$GHOSTTY_SHELL_INTEGRATION_XDG_DIR\"\n",
        );
        let root = "/integration root";
        let rearm = fish_rearm(root);
        let printer_path = printer.display().to_string();
        let shell = FinalShell {
            path: &printer_path,
            kind: LoginShell::Fish,
            rearm: &rearm,
        };
        let repeated = format!("{root}:/new/one:{root}");
        let cases: [(&[&str], String); 6] = [
            (
                &["set", "-gx", "XDG_DATA_DIRS", "/new/one:/new/two"],
                format!("{root}:/new/one:/new/two"),
            ),
            (
                &["set", "-gx", "XDG_DATA_DIRS", ""],
                format!("{root}:/usr/local/share:/usr/share"),
            ),
            (
                &["set", "-e", "XDG_DATA_DIRS"],
                format!("{root}:/usr/local/share:/usr/share"),
            ),
            (&["set", "-gx", "XDG_DATA_DIRS", &repeated], format!("{root}:/new/one")),
            (
                &["set", "-gx", "XDG_DATA_DIRS", "/new with space:/new/two"],
                format!("{root}:/new with space:/new/two"),
            ),
            // a command that leaves it alone keeps the session's own, empty entries dropped
            (&["true"], format!("{root}:/original/one:/original/two")),
        ];
        for (argv, expected) in cases {
            let output = run_fish(
                fish,
                &render_argv(argv, &shell),
                dir.path(),
                Some("/original/one::/original/two:"),
            );
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{expected}|{root}\n"),
                "{argv:?}"
            );
        }
    }

    fn pwsh_shell<'a>(rearm: &'a [Rearm], path: &'a str) -> FinalShell<'a> {
        FinalShell {
            path,
            kind: LoginShell::Pwsh,
            rearm,
        }
    }

    #[test]
    fn a_powershell_word_doubles_every_single_quote_powershell_reads() {
        assert_eq!(powershell_quoted("plain"), "'plain'");
        assert_eq!(powershell_quoted(""), "''");
        assert_eq!(powershell_quoted("it's"), "'it''s'");
        assert_eq!(
            powershell_quoted("\u{2018}curly\u{2019}"),
            "'\u{2018}\u{2018}curly\u{2019}\u{2019}'"
        );
        assert_eq!(
            powershell_quoted("\u{201a}low\u{201b}"),
            "'\u{201a}\u{201a}low\u{201b}\u{201b}'"
        );
        // nothing else is special inside single quotes: no variables, escapes or subexpressions
        assert_eq!(
            powershell_quoted(r#"$HOME `n "x" $(y) C:\dir\"#),
            r#"'$HOME `n "x" $(y) C:\dir\'"#
        );
    }

    #[test]
    fn powershell_rearms_and_loads_the_integration_before_the_command() {
        let rearm = [
            Rearm::Export("NGTERM_PWSH_INTEGRATION", r"C:\it's\ngterm.ps1".to_owned()),
            Rearm::Unset("STALE"),
        ];
        let shell = pwsh_shell(&rearm, r"C:\Program Files\PowerShell\7\pwsh.exe");
        let prefix =
            format!(r"$env:NGTERM_PWSH_INTEGRATION = 'C:\it''s\ngterm.ps1' ; $env:STALE = $null ; {PWSH_LOAD} ; ");
        // an argv the argument passings would hand on differently runs in a scope of its own, under the legacy
        // passing, its arguments quoted for the C runtime's split
        assert_eq!(
            render_argv(&[r"C:\tools\top.exe", "a b", "it's"], &shell),
            format!(
                "{prefix}& {{ $PSNativeCommandArgumentPassing = 'Legacy' ; & 'C:\\tools\\top.exe' '\"a b\"' 'it''s' }}"
            )
        );
        assert_eq!(
            render_argv(&["top", "-d", "1"], &shell),
            format!("{prefix}& 'top' '-d' '1'")
        );
        assert_eq!(
            render_command_line("Get-ChildItem | Select-Object -First 1; 'done'", &shell),
            format!("{prefix}. ([scriptblock]::Create('Get-ChildItem | Select-Object -First 1; ''done'''))")
        );
        // no exec: the shell `-NoExit` keeps is the final one, so its path is no part of the script
        assert!(!render_argv(&["top"], &shell).contains("pwsh.exe"));
    }

    /// PowerShell 7 when this host has one on `PATH` (the CI runners of every OS carry it): `None` otherwise.
    fn pwsh() -> Option<std::path::PathBuf> {
        let name = if cfg!(windows) { "pwsh.exe" } else { "pwsh" };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    }

    /// Where [`argv_recorder`] writes what it was run with.
    const RECORD: &str = "REPLAY_TEST_RECORD";

    /// Runs `script` under `pwsh` as a non-interactive command, with the live ngterm's variables stripped.
    fn run_pwsh(pwsh: &Path, script: &str, record: &Path) -> std::process::Output {
        let mut command = Command::new(pwsh);
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
            .env(RECORD, record)
            .env("STALE", "inherited")
            .stdin(std::process::Stdio::null());
        for (key, _) in std::env::vars_os() {
            if key
                .to_str()
                .is_some_and(|key| key.starts_with("NGTERM_") || key.starts_with("AGTERM_") || key.starts_with("ZMX_"))
            {
                command.env_remove(key);
            }
        }
        command.output().unwrap()
    }

    #[test]
    fn a_powershell_replay_round_trips_arguments_after_loading_the_integration() {
        let Some(pwsh) = pwsh() else {
            eprintln!("SKIPPED: no pwsh on PATH");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("record.json");
        // the stand-in integration marks what the command then inherits; its path needs quoting
        let integration = dir.path().join("it's integration.ps1");
        fs::write(&integration, "$env:REPLAY_TEST_LOADED = $env:NGTERM_PWSH_INTEGRATION\n").unwrap();
        let expected = [
            "a b",
            "it's \"quoted\"",
            r#"back\"slash"#,
            r"C:\dir with space\",
            "tab\there",
            "",
            "$HOME",
            "semi;colon",
            "\u{2018}curly\u{2019}",
        ];
        let mut argv = vec![
            std::env::current_exe().unwrap().display().to_string(),
            "--ignored".to_owned(),
            "--exact".to_owned(),
            "live::replay::tests::argv_recorder".to_owned(),
            "--".to_owned(),
        ];
        argv.extend(expected.iter().map(|arg| (*arg).to_owned()));
        let rearm = [
            Rearm::Export("NGTERM_PWSH_INTEGRATION", integration.display().to_string()),
            Rearm::Unset("STALE"),
        ];
        let pwsh_path = pwsh.display().to_string();
        let shell = pwsh_shell(&rearm, &pwsh_path);
        let output = run_pwsh(&pwsh, &render_argv(&argv, &shell), &record);
        assert!(output.status.success(), "{output:?}");
        let recorded: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        assert_eq!(
            recorded,
            serde_json::json!({
                "argv": expected,
                "loaded": integration.display().to_string(),
                "stale": null,
            })
        );
        // a profile's strict mode and legacy argument passing (which drops the empty argument and splits at the
        // quotes) change nothing, and the preference is the user's again after the command
        fs::remove_file(&record).unwrap();
        let preference = dir.path().join("preference");
        let script = format!(
            "Set-StrictMode -Version Latest ; $PSNativeCommandArgumentPassing = 'Legacy' ; {} ; \
             [System.IO.File]::WriteAllText({}, \"$PSNativeCommandArgumentPassing\")",
            render_argv(&argv, &shell),
            powershell_quoted(&preference.display().to_string())
        );
        let output = run_pwsh(&pwsh, &script, &record);
        assert!(output.status.success(), "{output:?}");
        let recorded: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        assert_eq!(recorded["argv"], serde_json::json!(expected));
        assert_eq!(fs::read_to_string(&preference).unwrap(), "Legacy");
        // a durable line keeps its statements and PowerShell's own word parsing
        let line = format!(
            "$words = 'two words'; Set-Content -LiteralPath {} -Value ($words + ' and more') -NoNewline",
            powershell_quoted(&dir.path().join("line").display().to_string())
        );
        let output = run_pwsh(&pwsh, &render_command_line(&line, &shell), &record);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            fs::read_to_string(dir.path().join("line")).unwrap(),
            "two words and more"
        );
    }

    /// The repository's PowerShell integration script.
    fn integration_script() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("resources")
            .join("shell-integration")
            .join("pwsh")
            .join("ngterm.ps1")
    }

    #[test]
    fn the_powershell_integration_loads_under_strict_mode_and_starts_the_prompt_ahead_of_its_host_output() {
        let Some(pwsh) = pwsh() else {
            eprintln!("SKIPPED: no pwsh on PATH");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        // a profile's strict mode, and its prompt writing to the host before it returns its text; then the prompt at
        // the start (no exit code yet) and after a command
        let script = format!(
            "Set-StrictMode -Version Latest ; \
             Microsoft.PowerShell.Utility\\Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction Ignore ; \
             function global:prompt {{ Microsoft.PowerShell.Utility\\Write-Host -NoNewline 'HOST|' ; 'PS> ' }} ; \
             $env:NGTERM_PWSH_INTEGRATION = {} ; {PWSH_LOAD} ; {PWSH_LOAD} ; \
             if (-not (Microsoft.PowerShell.Utility\\Get-Variable -Name __NgtermIntegration -Scope Global \
             -ErrorAction Ignore)) {{ exit 3 }} ; \
             [System.Console]::Write((prompt)) ; [System.Console]::Write('|') ; \
             $global:__NgtermIntegration.Running = $true ; [System.Console]::Write((prompt))",
            powershell_quoted(&integration_script().display().to_string())
        );
        let output = run_pwsh(&pwsh, &script, &dir.path().join("unused"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{output:?}");
        // a strict-mode error is no failure of the script, only its message
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(!stdout.contains("did not load"), "{stdout:?}");
        let prompt = "\u{1b}]133;A\u{7}HOST|PS> \u{1b}]133;B\u{7}";
        let (first, second) = stdout
            .split_once(&format!("{prompt}|"))
            .unwrap_or_else(|| panic!("{stdout:?}"));
        assert!(first.contains("\u{1b}]7;kitty-shell-cwd://"), "{stdout:?}");
        assert!(!first.contains("133;D"), "no command ran: {stdout:?}");
        assert!(
            second.contains("\u{1b}]133;D;0\u{7}") && second.ends_with(prompt),
            "{stdout:?}"
        );
    }

    /// Not a test of its own: the program the PowerShell replay test runs, which records its arguments after `--`
    /// and the variables the replay set, as JSON in the file [`RECORD`] names.
    #[test]
    #[ignore = "the PowerShell replay test's child"]
    fn argv_recorder() {
        let Some(record) = std::env::var_os(RECORD) else {
            return;
        };
        let argv: Vec<String> = std::env::args().skip_while(|arg| arg != "--").skip(1).collect();
        let recorded = serde_json::json!({
            "argv": argv,
            "loaded": std::env::var("REPLAY_TEST_LOADED").ok(),
            "stale": std::env::var("STALE").ok(),
        });
        fs::write(record, recorded.to_string()).unwrap();
    }
}
