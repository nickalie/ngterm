use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN_LIMIT: usize = 4096;

pub(crate) fn local_token() -> Option<String> {
    resolve(
        |name| std::env::var(name).ok(),
        || {
            let mut command = Command::new("gh");
            command
                .args(["auth", "token", "--hostname", "github.com"])
                .env("GH_PROMPT_DISABLED", "1");
            command_token(&mut command, Duration::from_secs(3))
        },
    )
}

fn resolve(env: impl Fn(&str) -> Option<String>, cli: impl FnOnce() -> Option<String>) -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .find_map(|name| env(name).as_deref().and_then(valid_token))
        .or_else(cli)
}

fn valid_token(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= TOKEN_LIMIT && value.bytes().all(|byte| byte.is_ascii_graphic()))
        .then(|| value.to_owned())
}

fn command_token(command: &mut Command, timeout: Duration) -> Option<String> {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        command,
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take((TOKEN_LIMIT + 1) as u64).read_to_end(&mut bytes);
        let _ = send.send(result.ok().map(|_| bytes));
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let bytes = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()??;
    valid_token(&String::from_utf8(bytes).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_tokens_take_precedence_and_invalid_values_are_skipped() {
        assert_eq!(
            resolve(|_| Some("first".into()), || panic!("unneeded CLI")),
            Some("first".into())
        );
        assert_eq!(
            resolve(|key| (key == "GITHUB_TOKEN").then(|| "second".into()), || None),
            Some("second".into())
        );
        assert_eq!(
            resolve(|_| Some("bad\nheader".into()), || Some("cli".into())),
            Some("cli".into())
        );
        assert_eq!(resolve(|_| None, || None), None);
        assert_eq!(valid_token(" token\r\n"), Some("token".into()));
        for value in [
            String::new(),
            " \n".into(),
            "a b".into(),
            "токен".into(),
            "x".repeat(TOKEN_LIMIT + 1),
        ] {
            assert!(valid_token(&value).is_none());
        }
    }

    fn shell(script: &str) -> Command {
        let mut command = if cfg!(windows) {
            Command::new("cmd.exe")
        } else {
            Command::new("sh")
        };
        command.args(if cfg!(windows) {
            vec!["/d", "/c", script]
        } else {
            vec!["-c", script]
        });
        command
    }

    #[test]
    fn command_success_failure_and_missing_executable_are_bounded() {
        assert_eq!(
            command_token(&mut shell("echo test-token"), Duration::from_secs(3)),
            Some("test-token".into())
        );
        assert_eq!(command_token(&mut shell("exit 1"), Duration::from_secs(3)), None);
        assert_eq!(
            command_token(
                &mut Command::new("ngterm-missing-credential-command"),
                Duration::from_secs(3)
            ),
            None
        );
        let script = if cfg!(windows) {
            "for /L %i in (1,0,2) do @rem wait"
        } else {
            "while :; do :; done"
        };
        let started = Instant::now();
        assert_eq!(command_token(&mut shell(script), Duration::from_millis(100)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
