//! A Mac running upstream agterm, as the phone reaches it over ssh: the conformance suite's stand-in for agterm
//! v0.41.1 (`conformance/agterm_conformance/fake_agterm.py`: its `agtermctl` and its bundled `zmx`, whose window leads
//! every pane it lists) behind an unprivileged `sshd` of its own, seeded with one live session. Its logins get a home
//! and a `PATH` holding only agterm's CLI: nothing of this build's runs there, and no live agterm is involved.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use ngterm_mobile::KeyPair;
use ngterm_mobile::api::generate_key;

use crate::machine::{account, quoted};
use crate::sshd::{Sshd, SshdSetup};

/// The session the stand-in is seeded with.
pub(crate) const SESSION: &str = "e2e-agterm-work";

const SEED: &str = r#"
import importlib.util, os, sys, uuid
spec = importlib.util.spec_from_file_location("fake_agterm", sys.argv[1])
fake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fake)
root, name = sys.argv[2], sys.argv[3]
daemon = f"agterm-{uuid.uuid4().hex}"
window = {"id": str(uuid.uuid4()).upper(), "name": "Main"}
workspace = {"id": str(uuid.uuid4()).upper(), "name": "work"}
session = {
    "id": str(uuid.uuid4()).upper(), "name": name,
    "windowID": window["id"], "windowName": window["name"],
    "workspaceID": workspace["id"], "workspaceName": workspace["name"],
    "cwd": os.path.join(root, "home"), "panes": [{"pane": "left", "daemon": daemon}],
}
fake.save_state(fake.Path(root), {
    "running": True, "restore": "live", "home": os.path.join(root, "home"),
    "zmx": os.path.join(root, "agterm.app", "zmx"), "socketDirectory": os.path.join(root, "zmx-sockets"),
    "window": window, "workspace": workspace, "sessions": [session], "daemons": [daemon],
})
"#;

/// The stand-in and the `sshd` in front of it, stopped when dropped.
pub(crate) struct AgtermMac {
    pub(crate) sshd: Sshd,
    pub(crate) key: KeyPair,
    pub(crate) host_fingerprint: String,
    directory: PathBuf,
}

impl AgtermMac {
    /// Lays the stand-in out under `work` and starts its `sshd` (`binary`).
    pub(crate) fn start(binary: &Path, work: &Path) -> Self {
        let directory = work.join("agterm");
        for part in ["bin", "agterm.app", "home", "zmx-sockets", "sshd"] {
            fs::create_dir_all(directory.join(part)).expect("create the stand-in's directories");
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/agterm_conformance/fake_agterm.py");
        let script = script.canonicalize().expect("the conformance suite's stand-in agterm");
        let python = python();
        for (program, wrapper) in [("agtermctl", "bin/agtermctl"), ("zmx", "agterm.app/zmx")] {
            let text = format!(
                "#!/bin/sh\nexec {} -I {} {} {program} \"$@\"\n",
                quoted(&python.display().to_string()),
                quoted(&script.display().to_string()),
                quoted(&directory.display().to_string()),
            );
            executable(&directory.join(wrapper), &text);
        }
        let seeded = Command::new(&python)
            .args(["-I", "-c", SEED])
            .arg(&script)
            .arg(&directory)
            .arg(SESSION)
            .status()
            .expect("run python3");
        assert!(seeded.success(), "seeding the stand-in agterm failed");
        let force_command = directory.join("force-command.sh");
        let text = format!(
            "#!/bin/sh\nexport HOME={}\nexport PATH={}\nexec /bin/sh -c \"$SSH_ORIGINAL_COMMAND\"\n",
            quoted(&directory.join("home").display().to_string()),
            quoted(&format!("{}:/usr/bin:/bin", directory.join("bin").display())),
        );
        executable(&force_command, &text);
        let key = generate_key("ngterm-mobile e2e agterm".to_owned()).expect("a client key");
        let host = generate_key("ngterm-mobile e2e agterm host".to_owned()).expect("a host key");
        let sshd = Sshd::start(&SshdSetup {
            binary,
            directory: &directory.join("sshd"),
            user: &account(),
            host_key: &host.private_key,
            authorized_key: &key.authorized_key,
            force_command: &force_command,
        })
        .unwrap_or_else(|error| panic!("start the agterm sshd: {error}"));
        Self {
            sshd,
            key,
            host_fingerprint: host.fingerprint,
            directory,
        }
    }

    /// Every `zmx` call a viewer made there so far, as the stand-in logged it.
    pub(crate) fn zmx_calls(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.directory.join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|call| call["program"] == "zmx")
            .collect()
    }
}

/// The system's `python3`, which the stand-in runs on.
fn python() -> PathBuf {
    ["/usr/bin/python3", "/bin/python3"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .expect("the stand-in agterm runs on python3, in /usr/bin or /bin")
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).expect("write a script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make it executable");
}
