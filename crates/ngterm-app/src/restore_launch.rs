//! The restore decision this launch takes before its first terminal (upstream's `GhosttyApp.init` latching
//! `restoreLaunchDecision` from the settings and `ZmxLaunch.liveUnavailableReason`), and the host facts the foreground
//! reads need: the user's `$SHELL`. The live probe's inputs (the staged `ngterm-session`, the password-database login
//! shell, the shell ghostty `command` names in its place, their integration) are [`Runtime`]'s, which the launch
//! captures once and hands to the claims and the wrap as well.

use ngterm_core::command_restore::basename;
use ngterm_core::paths::Env;
use ngterm_core::persist::AppSettings;
use ngterm_core::restore::mode::RestoreLaunchDecision;
use ngterm_session::launch::Runtime;

/// The decision for this launch: the settings' mode, `live` falling back to fresh shells with the probe of the launch's
/// capture `runtime`'s reason. Capture it after libghostty's resources dir is exported.
pub fn launch_decision(settings: &AppSettings, runtime: &Runtime, env: &dyn Env) -> RestoreLaunchDecision {
    let probe = runtime.live_probe(env);
    let decision = RestoreLaunchDecision::new(settings.effective_restore_mode(), probe.unavailable_reason());
    tracing::info!(
        requested = decision.requested.wire_name(),
        active = decision.active.wire_name(),
        reason = decision.live_unavailable_reason.as_deref().unwrap_or(""),
        command_shell = runtime.command_shell.as_deref().unwrap_or(""),
        "restore mode for this launch"
    );
    decision
}

/// The basename of the user's `$SHELL`, which widens shell recognition in the foreground reads.
pub fn shell_basename(env: &dyn Env) -> Option<String> {
    let shell = env.var("SHELL")?;
    let shell = shell.to_string_lossy();
    (!shell.is_empty()).then(|| basename(&shell).to_owned())
}

#[cfg(test)]
mod tests {
    use ngterm_core::paths::MapEnv;
    use ngterm_core::persist::RestoreMode;

    use super::*;

    #[test]
    fn a_live_request_without_a_bundled_session_server_falls_back_with_the_reason() {
        let settings = AppSettings {
            restore_mode: Some(RestoreMode::Live),
            ..AppSettings::default()
        };
        let env = MapEnv::new();
        let decision = launch_decision(&settings, &Runtime::capture(&env), &env);
        assert_eq!(decision.active, RestoreMode::None);
        assert_eq!(
            decision.live_unavailable_reason.as_deref(),
            Some("the live-session executable is unavailable")
        );
    }

    /// The decision is the handed capture's: a capture live only through the shell ghostty `command` names (#895) goes
    /// live, while the env's config chain names no such shell and a capture of this test process would find no staged
    /// session host beside it.
    #[cfg(unix)]
    #[test]
    fn the_decision_is_taken_over_the_launchs_capture() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("ngterm-session");
        ngterm_host::fakes::write_executable(&executable, "#!/bin/sh\n").unwrap();
        let resources = root.path().join("ghostty");
        let loader = resources.join("shell-integration/fish/vendor_conf.d/ghostty-shell-integration.fish");
        std::fs::create_dir_all(loader.parent().unwrap()).unwrap();
        std::fs::write(loader, "").unwrap();
        let state = root.path().join("state");
        let config = state.join("config").join("ghostty.conf");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "command = /usr/bin/tmux new\n").unwrap();
        let env = MapEnv::new()
            .with("NGTERM_STATE_DIR", &state)
            .with("HOME", root.path().join("home"));
        let mut capture = Runtime {
            executable,
            login_shell: Some("/bin/tcsh".to_owned()),
            command_shell: Some("/opt/homebrew/bin/fish".to_owned()),
            resources_dir: Some(resources),
            state_dir: state,
            ..Runtime::default()
        };
        let settings = AppSettings {
            restore_mode: Some(RestoreMode::Live),
            ..AppSettings::default()
        };
        let decision = launch_decision(&settings, &capture, &env);
        assert_eq!(
            (decision.active, decision.live_unavailable_reason),
            (RestoreMode::Live, None)
        );
        capture.command_shell = None;
        assert_eq!(
            launch_decision(&settings, &capture, &env)
                .live_unavailable_reason
                .as_deref(),
            Some(ngterm_core::restore::mode::UNSUPPORTED_LOGIN_SHELL)
        );
    }

    #[test]
    fn the_shell_basename_comes_from_shell() {
        assert_eq!(
            shell_basename(&MapEnv::new().with("SHELL", "/usr/bin/fish")).as_deref(),
            Some("fish")
        );
        assert_eq!(shell_basename(&MapEnv::new().with("SHELL", "")), None);
        assert_eq!(shell_basename(&MapEnv::new()), None);
    }
}
