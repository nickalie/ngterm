//! The ssh lines that ride a remote window's shared connection (upstream's pooled `RemoteSessionTests` cases).

use ngterm_proto::ControlLiveEndpoint;

use super::{
    PaneLabel, attach_pane_command, pooled, pooled_attach_pane_command, present_command, waiting_for_connection,
};
use crate::live::daemon_name;
use crate::live::lead::LeadAttachment;
use crate::remote::peer::RemotePeer;

fn endpoint() -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        peer: None,
        attach_bridge: None,
        executable: "/opt/ngterm/bin/ngterm-session".to_owned(),
        socket_directory: "/tmp/ngterm-live-abc".to_owned(),
    }
}

const LABEL: PaneLabel<'static> = PaneLabel {
    session: "s",
    pane: "left",
};

fn lead() -> LeadAttachment {
    LeadAttachment {
        nonce: "n1".to_owned(),
        claim: false,
    }
}

#[test]
fn a_pooled_command_rides_its_connection_and_an_unpooled_one_is_unchanged() {
    let plain = present_command("buildbox", RemotePeer::Ngterm, "0F1E2D3C").unwrap();

    let shared = pooled(plain.clone(), "/tmp/p-0");

    assert_eq!(
        shared[..13],
        [
            "ssh",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPath=/tmp/p-0",
            "-o",
            "ControlPersist=60",
            "buildbox"
        ]
    );
    assert_eq!(shared.last(), plain.last());
}

#[test]
fn an_unpooled_pane_script_has_no_connection_wait() {
    let line = attach_pane_command(
        "buildbox",
        &endpoint(),
        &daemon_name(uuid::Uuid::nil()),
        LABEL,
        Some(&lead()),
    )
    .unwrap();

    assert!(!line.contains("ngterm_connection_wait"));
    assert!(!line.contains("ControlPath"));
}

#[test]
fn a_job_helper_waits_for_its_connection_or_says_there_is_none() {
    let line = waiting_for_connection(&["ssh".to_owned(), "job".to_owned()], "buildbox", "/tmp/p-0");

    assert_eq!(line[..2], ["/bin/sh", "-c"]);
    assert!(line[2].contains("ngterm_connection_wait || {"), "{line:?}");
    assert!(line[2].contains("no connection to buildbox"));
    assert!(line[2].contains("exit 255"));
    assert!(line[2].ends_with("exec 'ssh' 'job'"), "{line:?}");
}

#[cfg(unix)]
mod run {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    use super::*;

    /// Runs `script` with an `ssh` whose master check exits `check` and whose attach prints `attached`; its output and
    /// status.
    fn run(argv: &[&str], check: i32) -> (String, Option<i32>) {
        let bin = tempfile::tempdir().unwrap();
        let ssh = bin.path().join("ssh");
        std::fs::write(
            &ssh,
            format!(
                "#!/bin/sh\ncase \"$*\" in *\"-O check\"*) exit {check};; *\" -G \"*) exit 0;; esac\necho attached; exit 0\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let output = Command::new(argv[0])
            .args(&argv[1..])
            .env("PATH", format!("{}:/usr/bin:/bin", bin.path().display()))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.code(),
        )
    }

    #[test]
    fn a_pooled_pane_attaches_once_its_connection_answers() {
        let line = pooled_attach_pane_command(
            "buildbox",
            &endpoint(),
            &daemon_name(uuid::Uuid::nil()),
            LABEL,
            None,
            Some("/tmp/p-0"),
        )
        .unwrap();

        let (output, status) = run(&["/bin/sh", "-c", &line], 0);

        assert!(output.contains("attached"), "{output}");
        assert_eq!(status, Some(0));
        assert!(line.contains("if ngterm_connection_wait; then"), "{line}");
        assert!(
            line.contains("else status=255; fi"),
            "no master is a lost connection, which takes the reconnect path"
        );
    }

    #[test]
    fn a_stream_with_no_master_says_so_and_exits_as_a_lost_connection() {
        let line = waiting_for_connection(&["ssh".to_owned(), "box".to_owned()], "buildbox", "/tmp/p-0");
        let script = line[2].replace("-ge 75", "-ge 1");

        let (output, status) = run(&["/bin/sh", "-c", &script], 255);

        assert_eq!(output, "ngterm: no connection to buildbox\n");
        assert_eq!(status, Some(255));
    }
}
