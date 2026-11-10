//! `ngtermctl viewer-run` reached as a Windows pane reaches it: its command line through `cmd.exe /D /S /C "<line>"`
//! (`ngterm_host::ShellCommand::cmd`), the helper named by its path in folders whose names `cmd.exe` would otherwise
//! split or expand.

#![cfg(windows)]

use std::os::windows::process::CommandExt as _;
use std::process::Command;

use ngterm_core::remote::viewer_run::ViewerRun;

#[test]
fn a_pane_line_reaches_the_helper_beside_the_app_from_an_awkward_folder() {
    let root = tempfile::tempdir().unwrap();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    for folder in ["a b & (c) ^d", "100% sure"] {
        let bin = root.path().join(folder);
        std::fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("ngtermctl.exe");
        std::fs::copy(env!("CARGO_BIN_EXE_ngtermctl"), &cli).unwrap();
        let run = ViewerRun::Program {
            argv: ["cmd.exe", "/d", "/c", "exit 7"].map(str::to_owned).to_vec(),
        };
        let line = run.line_for(&cli);
        let status = Command::new("cmd.exe")
            .args(["/D", "/S", "/C"])
            .raw_arg(format!("\"{line}\""))
            // no ngtermctl on PATH: only the named helper can answer
            .env("PATH", format!(r"{system}\System32"))
            .env("GHOSTTY_BIN_DIR", &bin)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(7), "{folder}: {line}");
    }
}
