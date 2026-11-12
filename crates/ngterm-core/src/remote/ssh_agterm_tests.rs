//! The lines a viewer runs against an agterm origin: its own CLI and zmx, reached the same way from any viewer OS.

use uuid::Uuid;

use super::*;

fn agterm_endpoint() -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        attach_bridge: Some(crate::attach_bridge::VERSION),
        executable: "/Applications/agterm.app/Contents/MacOS/zmx".to_owned(),
        socket_directory: "/tmp/agterm-zmx-1a2b".to_owned(),
        peer: Some(RemotePeer::Agterm),
    }
}

fn agterm_daemon() -> String {
    format!("agterm-{}", Uuid::from_u128(0x42).simple())
}

fn lead() -> LeadAttachment {
    LeadAttachment {
        nonce: "n1".to_owned(),
        claim: true,
    }
}

const AGTERM_FORM: OriginForm = OriginForm {
    shell: OriginShell::Posix,
    peer: RemotePeer::Agterm,
};

const AGTERM_CHAIN: &str = r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin:/opt/homebrew/bin" && "#;

#[test]
fn an_agterm_origins_stream_and_control_lines_run_agtermctl_with_its_zmx_group() {
    assert_eq!(
        present_tail(AGTERM_FORM, "S1").unwrap(),
        format!("{AGTERM_CHAIN}exec agtermctl '\\''zmx'\\'' '\\''present'\\'' '\\''S1'\\'''")
    );
    assert_eq!(present_tail(AGTERM_FORM, "s 1"), Err(InvocationError::InvalidSession));
    let control = control_tail(AGTERM_FORM, &["live", "list"]);
    assert!(control.contains("agtermctl"), "{control}");
    assert!(!control.contains("ngtermctl"), "{control}");
    // a Windows shell never pairs with agterm, which runs only on Macs: the POSIX form still goes
    let windows = OriginForm {
        shell: OriginShell::Windows,
        peer: RemotePeer::Agterm,
    };
    assert_eq!(control_tail(windows, &["live", "list"]), control);
    assert!(present_tail(windows, "S1").unwrap().starts_with(AGTERM_CHAIN));
}

#[cfg(unix)]
#[test]
fn an_agterm_control_line_reaches_agtermctl_with_every_word_intact() {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = "#!/bin/sh\nfor word in agtermctl \"$@\"; do printf '[%s]' \"$word\"; done\n";
    ngterm_host::fakes::write_executable(&bin.join("agtermctl"), script).unwrap();
    for (arguments, expected) in [
        (vec!["live", "list"], "[agtermctl][zmx][list][--json]"),
        (
            vec!["session", "new", "--no-select", "--name=it's \\ live"],
            "[agtermctl][session][new][--no-select][--name=it's \\ live][--json]",
        ),
    ] {
        let tail = control_tail(AGTERM_FORM, &arguments);
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &tail])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{output:?}");
    }
}

#[test]
fn a_pinned_host_sends_every_line_to_agterm_and_ssh_only_its_destination() {
    let pinned = peer::peer_of("agterm:me@studio");
    assert_eq!(pinned, RemotePeer::Agterm);
    let present = present_command("agterm:me@studio", pinned, "S1").unwrap();
    assert_eq!(present[6], "me@studio");
    assert_eq!(present[7], present_tail(AGTERM_FORM, "S1").unwrap());
    let job = run_job_command("agterm:me@studio", pinned, "J1").unwrap();
    assert_eq!(job[1], "-tt");
    assert_eq!(job[6], "me@studio");
    assert!(
        job[7].ends_with("exec agtermctl '\\''session'\\'' '\\''overlay'\\'' '\\''run-job'\\'' '\\''J1'\\'''"),
        "{}",
        job[7]
    );
    let page = page_serve_command("agterm:me@studio", pinned, "T1").unwrap();
    assert_eq!(page[1], "-T");
    assert!(
        page[7].contains("agtermctl '\\''session'\\'' '\\''overlay'\\'' '\\''page-serve'\\''"),
        "{}",
        page[7]
    );
    let control = control_command("agterm:me@studio", pinned, &["session", "close", "--target", "S1"]).unwrap();
    assert_eq!(control[6], "me@studio");
    assert_eq!(
        control[7],
        control_tail(AGTERM_FORM, &["session", "close", "--target", "S1"])
    );
    assert_eq!(probe_command("agterm:me@studio").unwrap()[6], "me@studio");
}

#[test]
fn a_line_goes_to_the_peer_it_is_given_whatever_the_host_last_listed() {
    let host = "ssh-agterm-learned.example";
    peer::learn(host, RemotePeer::Ngterm);
    assert_eq!(
        present_command(host, RemotePeer::Agterm, "S1").unwrap()[7],
        present_tail(AGTERM_FORM, "S1").unwrap()
    );
    assert_eq!(present_command(host, RemotePeer::Agterm, "S1").unwrap()[6], host);
    peer::learn(host, RemotePeer::Agterm);
    assert_eq!(
        present_command(host, RemotePeer::Ngterm, "S1").unwrap()[7],
        present_tail(OriginShell::Posix, "S1").unwrap()
    );
    assert_eq!(
        control_command(host, RemotePeer::Ngterm, &["live", "list"]).unwrap()[7],
        control_tail(OriginShell::Posix, &["live", "list"])
    );
}

#[test]
fn an_agterm_pane_attaches_its_zmx_under_zmx_names_with_no_bridge() {
    let daemon = agterm_daemon();
    let guard = r"'/bin/sh' '-c' 'printf '\''%s\n'\'' '\''ngterm: remote session is gone'\''; exit 1'";
    let with_lead = format!(
        "'/usr/bin/env' 'ZMX_SESSION=' 'ZMX_SESSION_PREFIX=' 'ZMX_NO_DETACH_KEY=1' 'ZMX_DIR=/tmp/agterm-zmx-1a2b' \
         'ZMX_MANAGED=n1' 'ZMX_MANAGED_CLAIM=1' '/Applications/agterm.app/Contents/MacOS/zmx' 'attach' '{daemon}' \
         {guard}"
    );
    assert_eq!(
        attach_tail(&agterm_endpoint(), &daemon, Some(&lead())).unwrap(),
        with_lead
    );
    let without = attach_tail(&agterm_endpoint(), &daemon, None).unwrap();
    assert!(!without.contains("ZMX_MANAGED"), "{without}");
    assert!(!without.contains("NGTERM_"), "{without}");
    let argv = attach_command("agterm:studio", &agterm_endpoint(), &daemon, Some(&lead())).unwrap();
    assert_eq!(
        argv[..9],
        [
            "ssh",
            "-tt",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "LogLevel=ERROR",
            "studio"
        ]
    );
    assert_eq!(argv[9], with_lead);
}

#[test]
fn a_daemon_name_must_be_its_own_peers() {
    let ngterm_daemon = crate::live::daemon_name(Uuid::from_u128(0x42));
    assert_eq!(
        attach_tail(&agterm_endpoint(), &ngterm_daemon, None),
        Err(InvocationError::InvalidSession)
    );
    let mut ngterm = agterm_endpoint();
    ngterm.peer = None;
    assert_eq!(
        attach_tail(&ngterm, &agterm_daemon(), None),
        Err(InvocationError::InvalidSession)
    );
    let mut hostile = agterm_endpoint();
    hostile.socket_directory = "/tmp/x\n".to_owned();
    assert_eq!(
        attach_tail(&hostile, &agterm_daemon(), None),
        Err(InvocationError::InvalidEndpoint)
    );
}

/// The attach as the agterm machine's shell runs it: a stand-in zmx at the endpoint's path sees zmx's variables and
/// none of this build's.
#[cfg(unix)]
#[test]
fn the_agterm_attach_hands_its_zmx_exactly_zmxs_contract() {
    let root = tempfile::tempdir().unwrap();
    let zmx = root.path().join("My Apps/zmx");
    std::fs::create_dir_all(zmx.parent().unwrap()).unwrap();
    let script = "#!/bin/sh\nenv | grep -E '^(ZMX|NGTERM)_' | sort\nprintf '%s\\n' \"$@\"\n";
    ngterm_host::fakes::write_executable(&zmx, script).unwrap();
    let mut endpoint = agterm_endpoint();
    endpoint.executable = zmx.to_string_lossy().into_owned();
    let tail = attach_tail(&endpoint, &agterm_daemon(), Some(&lead())).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &tail])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("ZMX_SESSION", "inherited")
        .env("NGTERM_LIVE_SESSION", "inherited")
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&output.stdout);
    let expected = format!(
        "NGTERM_LIVE_SESSION=inherited\nZMX_DIR=/tmp/agterm-zmx-1a2b\nZMX_MANAGED=n1\nZMX_MANAGED_CLAIM=1\n\
         ZMX_NO_DETACH_KEY=1\nZMX_SESSION=\nZMX_SESSION_PREFIX=\nattach\n{}\n/bin/sh\n-c\n{GONE_GUARD}\n",
        agterm_daemon()
    );
    assert_eq!(printed, expected, "{output:?}");
}

/// A Windows viewer runs no `/bin/sh`: its pane's line is `ngtermctl viewer-run WORD`, whose record carries the same
/// ssh argv, the agterm tail included, so nothing of agterm runs on the viewer. A Unix viewer's script quotes that argv.
#[test]
fn every_viewer_os_sends_the_agterm_attach_as_one_ssh_argv() {
    let daemon = agterm_daemon();
    let label = PaneLabel {
        session: "build",
        pane: "left",
    };
    let line = attach_pane_command("agterm:studio", &agterm_endpoint(), &daemon, label, Some(&lead())).unwrap();
    let argv = attach_command("agterm:studio", &agterm_endpoint(), &daemon, Some(&lead())).unwrap();
    if cfg!(windows) {
        let word = line.rsplit(' ').next().unwrap();
        assert!(
            line.contains(&format!(" {} ", super::super::viewer_run::VIEWER_RUN)),
            "{line}"
        );
        let ViewerRun::Pane { attach, report, lost } = ViewerRun::decode(word).unwrap() else {
            panic!("a pane record: {line}");
        };
        assert_eq!(attach, argv);
        assert_eq!(report, "ngterm: build (left) on agterm:studio disconnected, exit");
        assert!(lost.is_some());
    } else {
        assert!(line.starts_with("'/usr/bin/env' '/bin/sh' '-c' "), "{line}");
        let tail = argv.last().unwrap();
        assert!(line.contains(&shell_quoted_line(&[tail.as_str()])[1..20]), "{line}");
        assert!(line.contains("studio"), "{line}");
    }
    assert_eq!(argv[8], "studio");
}
