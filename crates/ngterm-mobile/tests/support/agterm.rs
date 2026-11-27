//! A Mac running upstream agterm, as the phone reaches it over ssh: the conformance suite's stand-in for agterm
//! v0.41.1 (`conformance/agterm_conformance/fake_agterm.py`: its `agtermctl` and its bundled `zmx`, whose window leads
//! every pane it lists; [`StandIn`]) behind an unprivileged `sshd` of its own, seeded with one live session. Its logins
//! get a home and a `PATH` holding only agterm's CLI: nothing of this build's runs there, and no live agterm is
//! involved. The stand-in alone is also the agterm of a machine that runs both apps (`both.rs`).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use ngterm_mobile::KeyPair;
use ngterm_mobile::api::generate_key;

use crate::machine::{account, quoted, text};
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

/// The stand-in agterm laid out in a directory: `bin/agtermctl`, `agterm.app/zmx` and its state, with one live
/// session.
pub(crate) struct StandIn {
    directory: PathBuf,
}

impl StandIn {
    /// Lays the stand-in out in `directory`, seeded with the live session `session`.
    pub(crate) fn lay_out(directory: &Path, session: &str) -> Self {
        for part in ["bin", "agterm.app", "home", "zmx-sockets"] {
            fs::create_dir_all(directory.join(part)).expect("create the stand-in's directories");
        }
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/agterm_conformance/fake_agterm.py");
        let script = script.canonicalize().expect("the conformance suite's stand-in agterm");
        let python = python();
        for (program, wrapper) in [("agtermctl", "bin/agtermctl"), ("zmx", "agterm.app/zmx")] {
            // agterm is no program of this build: a coverage run's profile destination, which a login of a machine
            // running both apps carries for ngterm's programs, stops here, so nothing agterm starts counts as a writer
            // of that run's profiles
            let text = format!(
                "#!/bin/sh\nunset LLVM_PROFILE_FILE\nexec {} -I {} {} {program} \"$@\"\n",
                quoted(&python.display().to_string()),
                quoted(&script.display().to_string()),
                quoted(&directory.display().to_string()),
            );
            executable(&directory.join(wrapper), &text);
        }
        let seeded = Command::new(&python)
            .args(["-I", "-c", SEED])
            .arg(&script)
            .arg(directory)
            .arg(session)
            .status()
            .expect("run python3");
        assert!(seeded.success(), "seeding the stand-in agterm failed");
        Self {
            directory: directory.to_owned(),
        }
    }

    /// The directory of agterm's CLI, for a login's `PATH`.
    pub(crate) fn bin(&self) -> PathBuf {
        self.directory.join("bin")
    }

    /// The home its sessions start in.
    pub(crate) fn home(&self) -> PathBuf {
        self.directory.join("home")
    }

    /// Every `zmx` call a viewer made there so far, as the stand-in logged it.
    pub(crate) fn zmx_calls(&self) -> Vec<serde_json::Value> {
        self.calls("zmx")
    }

    /// The argv of every `agtermctl` call made there so far, as the stand-in logged it.
    pub(crate) fn agtermctl_calls(&self) -> Vec<Vec<String>> {
        self.calls("agtermctl")
            .iter()
            .map(|call| {
                call["argv"]
                    .as_array()
                    .map(|argv| argv.iter().map(text).collect())
                    .unwrap_or_default()
            })
            .collect()
    }

    fn calls(&self, program: &str) -> Vec<serde_json::Value> {
        fs::read_to_string(self.directory.join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|call| call["program"] == program)
            .collect()
    }

    /// Sets agterm's restore mode, as its Settings would for the next launch and that launch did.
    pub(crate) fn set_restore(&self, mode: &str) {
        let path = self.directory.join("state.json");
        let mut state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("the stand-in's state")).expect("its JSON");
        state["restore"] = serde_json::Value::from(mode);
        fs::write(&path, state.to_string()).expect("write the stand-in's state");
    }

    /// agterm's restore mode now.
    pub(crate) fn restore(&self) -> String {
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(self.directory.join("state.json")).expect("the stand-in's state"))
                .expect("its JSON");
        text(&state["restore"])
    }

    /// The names of the sessions agterm itself lists now (its own `agtermctl zmx tree --json`).
    pub(crate) fn sessions(&self) -> Vec<String> {
        let output = Command::new(self.bin().join("agtermctl"))
            .args(["zmx", "tree", "--json"])
            .output()
            .expect("run the stand-in's agtermctl");
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).expect("agterm's listing");
        reply["result"]["remote"]["sessions"]
            .as_array()
            .map(|sessions| sessions.iter().map(|session| text(&session["name"])).collect())
            .unwrap_or_default()
    }
}

/// The stand-in and the `sshd` in front of it, stopped when dropped.
pub(crate) struct AgtermMac {
    pub(crate) sshd: Sshd,
    pub(crate) key: KeyPair,
    pub(crate) host_fingerprint: String,
    stand_in: StandIn,
}

impl AgtermMac {
    /// Lays the stand-in out under `work` and starts its `sshd` (`binary`).
    pub(crate) fn start(binary: &Path, work: &Path) -> Self {
        let directory = work.join("agterm");
        let stand_in = StandIn::lay_out(&directory, SESSION);
        fs::create_dir_all(directory.join("sshd")).expect("create the sshd directory");
        let force_command = directory.join("force-command.sh");
        let text = format!(
            "#!/bin/sh\nexport HOME={}\nexport PATH={}\nexec /bin/sh -c \"$SSH_ORIGINAL_COMMAND\"\n",
            quoted(&stand_in.home().display().to_string()),
            quoted(&format!("{}:/usr/bin:/bin", stand_in.bin().display())),
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
            stand_in,
        }
    }

    /// Every `zmx` call a viewer made there so far ([`StandIn::zmx_calls`]).
    pub(crate) fn zmx_calls(&self) -> Vec<serde_json::Value> {
        self.stand_in.zmx_calls()
    }

    /// Every `agtermctl` call made there so far ([`StandIn::agtermctl_calls`]).
    pub(crate) fn agtermctl_calls(&self) -> Vec<Vec<String>> {
        self.stand_in.agtermctl_calls()
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

pub(crate) fn executable(path: &Path, text: &str) {
    fs::write(path, text).expect("write a script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make it executable");
}
