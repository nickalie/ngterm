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

    /// Whether a copied argv quoted for this profile's dialect runs its words literally where the profile runs it: a
    /// direct CMD profile ([`Self::direct_live`]) with delayed expansion on (`/V:ON`) reads a `!NAME!` the quoting
    /// leaves in an argument as a variable. Any other profile runs a copy through the platform's command shell, not
    /// through itself; a line typed into the profile's shell is [`Self::expands_typed_input`]'s.
    #[must_use]
    pub fn reads_quoted_argv(&self) -> bool {
        self.direct_live() != Some(DirectShell::Cmd) || !delayed_expansion(&self.args)
    }

    /// Whether a line typed into this profile's shell (a rerun's replay) may read a `!NAME!` the quoting leaves as a
    /// variable: any `cmd` profile whose startup switches turn delayed expansion on, Live-eligible or not.
    #[must_use]
    pub fn expands_typed_input(&self) -> bool {
        let name = self.executable.rsplit(['/', '\\']).next().unwrap_or_default();
        let name = name.to_ascii_lowercase();
        name.strip_suffix(".exe").unwrap_or(&name) == "cmd" && delayed_expansion(&self.args)
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
            if kind == DirectShell::Wsl || !should_restore(words, denylist) || delayed_expansion(&profile.args) {
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

/// Whether CMD startup switches `args` turn delayed expansion on, the last `/V` winning. Only the switches before the
/// command (`/C`, `/K` or `/R`) count, so a command's text is never read as one. cmd reads a `/V` that is not `/V:OFF`
/// as on and takes switches run together (`/D/V:ON`), so both count too. The registry's `DelayedExpansion` default is
/// not read: a replay cannot tell it.
fn delayed_expansion(args: &[String]) -> bool {
    let mut on = false;
    for switch in args
        .iter()
        .filter_map(|arg| arg.strip_prefix('/'))
        .flat_map(|switches| switches.split('/'))
        .map(str::to_ascii_lowercase)
    {
        match switch.chars().next() {
            Some('c' | 'k' | 'r') => break,
            Some('v') => on = switch != "v:off",
            _ => {}
        }
    }
    on
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

    #[test]
    fn only_cmd_switches_before_its_command_turn_on_delayed_expansion() {
        for (exe, args, expected) in [
            ("C:/Windows/System32/cmd.exe", vec!["/V:ON", "/K", "echo ready"], true),
            ("cmd", vec!["/D/V:ON", "/T:0A"], true),
            ("cmd.exe", vec!["/v"], true),
            ("cmd.exe", vec!["/V:ON", "/V:OFF"], false),
            ("cmd.exe", vec!["/K", "tool", "/V:ON"], false),
            ("cmd.exe", vec!["/R", "x", "/V:ON"], false),
            ("C:/Program Files/PowerShell/7/pwsh.exe", vec!["/V:ON"], false),
        ] {
            let profile = ShellProfile::new("test", "Test", exe.into(), &args);
            assert_eq!(profile.expands_typed_input(), expected, "{exe} {args:?}");
        }
    }
}
