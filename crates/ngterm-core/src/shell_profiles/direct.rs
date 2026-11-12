//! Native Windows programs kept alive by the session server.
//! WSL retains its distribution/user/directory arguments; the Windows server owns `wsl.exe`'s `ConPTY`.

use super::{ShellProfile, windows_absolute};
use crate::command_restore::{basename, should_restore};
use crate::live::support::Creation;
use crate::shell_dialect::ShellDialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectShell {
    Cmd,
    Wsl,
}

impl ShellProfile {
    /// Only startup options, never a one-shot command or a WSL management operation.
    #[must_use]
    pub fn direct_live(&self) -> Option<DirectShell> {
        if !self.valid() || !windows_absolute(&self.executable) {
            return None;
        }
        match basename(&self.executable).to_ascii_lowercase().as_str() {
            "cmd.exe" if self.args.iter().all(|arg| cmd_option(arg)) => Some(DirectShell::Cmd),
            "wsl.exe" if wsl_options(&self.args) => Some(DirectShell::Wsl),
            _ => None,
        }
    }
}

fn cmd_option(arg: &str) -> bool {
    matches!(
        arg.to_ascii_lowercase().as_str(),
        "/d" | "/q" | "/a" | "/u" | "/e:on" | "/e:off" | "/v:on" | "/v:off" | "/f:on" | "/f:off"
    )
}

fn wsl_options(args: &[String]) -> bool {
    let mut words = args.iter();
    while let Some(option) = words.next() {
        if !matches!(option.as_str(), "-d" | "--distribution" | "-u" | "--user" | "--cd") {
            return false;
        }
        if words
            .next()
            .is_none_or(|value| value.is_empty() || value.contains(['\r', '\n']))
        {
            return false;
        }
    }
    true
}

/// Add WSL shell startup integration when bundled, retaining the plain-shell fallback.
pub fn integrated_arguments<S: AsRef<str>>(
    profile: &ShellProfile,
    replaying: Option<&[String]>,
    creation: Option<Creation<'_>>,
    denylist: &[S],
    environment: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    if profile.direct_live() == Some(DirectShell::Wsl) {
        let payload = match (replaying, creation) {
            (None, Some(Creation::ReturningToShell(line))) => Some((line, true)),
            (None, Some(Creation::EndingPane(line))) => Some((line, false)),
            _ => None,
        };
        if let Some(integration) = super::integration::wsl_arguments(environment, payload) {
            let mut argv = vec![profile.executable.clone()];
            argv.extend(profile.args.iter().cloned());
            argv.extend(integration);
            return argv;
        }
    }
    arguments(profile, replaying, creation, denylist)
}

/// Create a persistent program, honoring explicit command lifetimes. A surviving daemon ignores this argv.
/// WSL's Linux foreground processes cannot be captured by the Windows process table: do not replay a captured
/// Windows-side wsl.exe command inside Linux when the server itself has ended.
pub fn arguments<S: AsRef<str>>(
    profile: &ShellProfile,
    replaying: Option<&[String]>,
    creation: Option<Creation<'_>>,
    denylist: &[S],
) -> Vec<String> {
    let mut argv = vec![profile.executable.clone()];
    argv.extend(profile.args.iter().cloned());
    let Some(kind) = profile.direct_live() else {
        return argv;
    };
    let (line, returning) = match (replaying, creation) {
        (Some(words), _) => {
            // cmd's quoting escapes `!` once, which delayed expansion (`/V:ON`) would read as `!NAME!` anyway: such a
            // shell gets no replay rather than a command whose arguments take variables' values
            if kind == DirectShell::Wsl || !should_restore(words, denylist) || delayed_expansion(profile) {
                return argv;
            }
            let Some(line) = ShellDialect::Cmd.line(words) else {
                return argv;
            };
            (line, true)
        }
        (None, Some(Creation::ReturningToShell(line))) => (line.to_owned(), true),
        (None, Some(Creation::EndingPane(line))) => (line.to_owned(), false),
        (None, None) => return argv,
    };
    match kind {
        DirectShell::Cmd => {
            argv.extend(["/S".to_owned(), if returning { "/K" } else { "/C" }.to_owned(), line]);
        }
        DirectShell::Wsl => {
            let script = if returning {
                format!("{line}\nexec \"${{SHELL:-/bin/sh}}\" -l")
            } else {
                line
            };
            argv.extend(["--exec", "/bin/sh", "-lc"].map(str::to_owned));
            argv.push(script);
        }
    }
    argv
}

/// Whether `profile`'s CMD switches turn delayed expansion on (`/V:ON`, the last `/V` winning). The registry's
/// `DelayedExpansion` default is not read: a replay cannot tell it.
fn delayed_expansion(profile: &ShellProfile) -> bool {
    profile
        .args
        .iter()
        .rev()
        .find_map(|arg| match arg.to_ascii_lowercase().as_str() {
            "/v:on" => Some(true),
            "/v:off" => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_interactive_cmd_and_wsl_profiles_are_live() {
        for (exe, args, expected) in [
            ("cmd.exe", vec!["/D", "/V:OFF"], Some(DirectShell::Cmd)),
            ("cmd.exe", vec!["/K", "echo hi"], None),
            ("cmd.exe", vec!["/C", "exit"], None),
            (
                "wsl.exe",
                vec![
                    "--distribution",
                    "Ubuntu Dev",
                    "--user",
                    "tester",
                    "--cd",
                    "/home/tester",
                ],
                Some(DirectShell::Wsl),
            ),
            ("wsl.exe", vec!["--shutdown"], None),
            ("wsl.exe", vec!["--distribution"], None),
            ("wsl.exe", vec!["--exec", "echo", "hi"], None),
        ] {
            let profile = ShellProfile::new("test", "Test", format!("C:/Windows/System32/{exe}"), &args);
            assert_eq!(profile.direct_live(), expected, "{args:?}");
        }
    }

    #[test]
    fn cmd_and_wsl_preserve_startup_arguments_and_creation_lifetime() {
        let cmd = ShellProfile::new("cmd", "CMD", "C:/Windows/System32/cmd.exe".into(), &["/D"]);
        let empty: [&str; 0] = [];
        let line = "echo \"two words\" & echo done";
        assert_eq!(
            arguments(&cmd, None, Some(Creation::EndingPane(line)), &empty),
            [cmd.executable.as_str(), "/D", "/S", "/C", line]
        );
        assert_eq!(
            arguments(&cmd, None, Some(Creation::ReturningToShell(line)), &empty)[3],
            "/K"
        );
        let wsl = ShellProfile::new(
            "wsl",
            "Ubuntu",
            "C:/Windows/System32/wsl.exe".into(),
            &["-d", "Ubuntu Dev", "--user", "tester"],
        );
        let login = arguments(&wsl, None, None, &empty);
        assert_eq!(&login[1..], wsl.args);
        let payload = arguments(&wsl, None, Some(Creation::EndingPane("printf '%s' 'a b'")), &empty);
        assert_eq!(&payload[1..5], wsl.args);
        assert_eq!(&payload[5..], ["--exec", "/bin/sh", "-lc", "printf '%s' 'a b'"]);
        assert_eq!(arguments(&wsl, Some(&login), None, &empty), login);
    }

    #[test]
    fn a_delayed_expansion_cmd_gets_no_replay_of_a_captured_command() {
        let empty: [&str; 0] = [];
        let captured = ["curl".to_owned(), "https://example.test/?q=!SECRET!".to_owned()];
        let plain = ShellProfile::new("cmd", "CMD", "C:/Windows/System32/cmd.exe".into(), &["/D"]);
        let replayed = arguments(&plain, Some(&captured), None, &empty);
        assert_eq!(replayed[2..4], ["/S", "/K"]);
        for args in [vec!["/D", "/V:ON"], vec!["/v:off", "/v:on"]] {
            let delayed = ShellProfile::new("cmd", "CMD", "C:/Windows/System32/cmd.exe".into(), &args);
            assert_eq!(
                arguments(&delayed, Some(&captured), None, &empty)[1..],
                delayed.args,
                "{args:?}"
            );
        }
        let off = ShellProfile::new("cmd", "CMD", "C:/Windows/System32/cmd.exe".into(), &["/V:ON", "/V:OFF"]);
        assert_eq!(arguments(&off, Some(&captured), None, &empty).len(), 6);
    }
}
