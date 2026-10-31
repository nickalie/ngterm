//! The agent skill's image helper (`resources/skills/ngterm/scripts/show-image.sh`, and on Windows `show-image.ps1`
//! beside it: `build/patches/skill-ngterm/0008-show-image-windows.patch`), run for real with a recording `ngtermctl`
//! first on `PATH`, which writes its arguments one per line: the overlay the script asks for, on the session's own
//! socket. On Unix that is a program overlay running the script itself in `--emit` mode, which draws the image with
//! the kitty graphics protocol in the overlay's PTY; on Windows, whose pseudoconsole drops those escapes, a page
//! overlay holding the image, from PowerShell and from Git Bash (which hands over to the PowerShell form) alike.
//! Without an image, or outside ngterm, the script refuses and asks for nothing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xc9, 0xfe, 0x92, 0xef,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const SOCKET: &str = "the session's socket";

/// A scratch root holding a copy of the scripts, the image and the recording `ngtermctl`; on Unix by its canonical
/// path, which is what the script resolves its own paths to (`cd` and `pwd`), on Windows as given, since a canonical
/// path there is a verbatim `\\?\` one.
struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Root {
    fn new() -> TestResult<Self> {
        let dir = tempfile::tempdir()?;
        #[cfg(unix)]
        let path = fs::canonicalize(dir.path())?;
        #[cfg(windows)]
        let path = dir.path().to_path_buf();
        let skill = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/skills/ngterm/scripts");
        fs::create_dir(path.join("scripts"))?;
        for script in ["show-image.sh", "show-image.ps1"] {
            fs::copy(skill.join(script), path.join("scripts").join(script))?;
        }
        fs::create_dir(path.join("bin"))?;
        fs::create_dir(path.join("temp"))?;
        fs::write(path.join("shot.png"), PNG)?;
        let root = Self { _dir: dir, path };
        root.write_recorder()?;
        Ok(root)
    }

    fn calls_file(&self) -> PathBuf {
        self.path.join("calls")
    }

    /// The arguments `ngtermctl` was called with, one per line; empty when it never ran.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.calls_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[cfg(unix)]
    fn write_recorder(&self) -> TestResult {
        use std::os::unix::fs::PermissionsExt as _;
        let recorder = self.path.join("bin/ngtermctl");
        let calls = self.calls_file().display().to_string().replace('\'', r"'\''");
        fs::write(
            &recorder,
            format!("#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\"; done >> '{calls}'\n"),
        )?;
        fs::set_permissions(&recorder, fs::Permissions::from_mode(0o755))?;
        Ok(())
    }

    /// A PowerShell script, which PowerShell runs in place of `ngtermctl.exe` with each argument kept whole.
    #[cfg(windows)]
    fn write_recorder(&self) -> TestResult {
        let calls = self.calls_file().display().to_string().replace('\'', "''");
        fs::write(
            self.path.join("bin/ngtermctl.ps1"),
            format!("$args | Add-Content -LiteralPath '{calls}'\r\nexit 0\r\n"),
        )?;
        Ok(())
    }

    /// `program` with `args`, in a pane's environment when `inside` (`NGTERM_ENABLED`, `NGTERM_SOCKET`), the recorder
    /// first on `PATH`.
    fn run(&self, program: &Path, args: &[&str], inside: bool) -> TestResult<Output> {
        let mut command = Command::new(program);
        command.args(args).current_dir(&self.path);
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().into_owned();
            if ["NGTERM_", "AGTERM_", "ZMX_"]
                .iter()
                .any(|prefix| key.starts_with(prefix))
            {
                command.env_remove(key);
            }
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![self.path.join("bin")];
        dirs.extend(std::env::split_paths(&path));
        command.env("PATH", std::env::join_paths(dirs)?);
        command
            .env("TEMP", self.path.join("temp"))
            .env("TMP", self.path.join("temp"));
        if inside {
            command.env("NGTERM_ENABLED", "1").env("NGTERM_SOCKET", SOCKET);
        }
        Ok(command.output()?)
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(unix)]
fn show_image(root: &Root, args: &[&str], inside: bool) -> TestResult<Output> {
    let script = root.path.join("scripts/show-image.sh");
    let mut line = vec![script.to_str().ok_or("a UTF-8 path")?];
    line.extend_from_slice(args);
    root.run(Path::new("bash"), &line, inside)
}

#[cfg(windows)]
fn show_image(root: &Root, args: &[&str], inside: bool) -> TestResult<Output> {
    let script = root.path.join("scripts").join("show-image.ps1");
    let mut line = vec![
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        script.to_str().ok_or("a UTF-8 path")?,
    ];
    line.extend_from_slice(args);
    root.run(Path::new("powershell.exe"), &line, inside)
}

#[cfg(unix)]
#[test]
fn the_image_opens_in_a_program_overlay_running_the_script_on_the_sessions_socket() -> TestResult {
    let root = Root::new()?;
    let image = root.path.join("shot.png");
    let output = show_image(&root, &[image.to_str().ok_or("a UTF-8 path")?, "40"], true)?;
    assert!(output.status.success(), "show-image.sh failed: {}", stderr(&output));
    let program = format!(
        "/bin/bash '{}' --emit '{}'",
        root.path.join("scripts/show-image.sh").display(),
        image.display()
    );
    assert_eq!(
        root.calls(),
        [
            "session",
            "overlay",
            "open",
            program.as_str(),
            "--size-percent",
            "40",
            "--socket",
            SOCKET
        ]
    );
    Ok(())
}

/// The page the Windows form wrote (`<TEMP>\ngterm-show-image-*\image.html`) is the one it opened, chromeless with its
/// script on, on the session's socket, and holds the image and the Enter that closes it.
#[cfg(windows)]
fn assert_page_opened(root: &Root, size: &str) -> TestResult {
    use base64::Engine as _;
    let calls = root.calls();
    assert_eq!(calls.len(), 11, "one overlay open: {calls:?}");
    let page = PathBuf::from(&calls[4]);
    assert_eq!(
        calls,
        [
            "session",
            "overlay",
            "open",
            "--html",
            calls[4].as_str(),
            "--js",
            "--chromeless",
            "--size-percent",
            size,
            "--socket",
            SOCKET
        ]
    );
    assert_eq!(page.file_name().and_then(std::ffi::OsStr::to_str), Some("image.html"));
    let folder = page.parent().ok_or("a folder")?;
    // compared canonical: PowerShell's TEMP has the long names of a short (8.3) one the test gave it
    assert_eq!(
        fs::canonicalize(folder.parent().ok_or("a folder")?)?,
        fs::canonicalize(root.path.join("temp"))?,
        "under TEMP: {}",
        page.display()
    );
    assert!(
        folder
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.starts_with("ngterm-show-image-")),
        "{}",
        page.display()
    );
    let html = fs::read_to_string(&page)?;
    let data = base64::engine::general_purpose::STANDARD.encode(PNG);
    assert!(
        html.contains(&format!("src=\"data:image/png;base64,{data}\"")),
        "{html}"
    );
    assert!(html.contains("press Enter to close"), "{html}");
    assert!(html.contains("ngterm.request('session.overlay.close')"), "{html}");
    assert!(html.contains("<title>shot.png</title>"), "{html}");
    Ok(())
}

#[cfg(windows)]
#[test]
fn the_image_opens_in_a_page_overlay_on_the_sessions_socket() -> TestResult {
    let root = Root::new()?;
    let image = root.path.join("shot.png");
    let output = show_image(&root, &[image.to_str().ok_or("a UTF-8 path")?, "40"], true)?;
    assert!(output.status.success(), "show-image.ps1 failed: {}", stderr(&output));
    assert_page_opened(&root, "40")
}

/// Git for Windows' bash, where its installer puts it, as an agent's shell finds it; none on a machine without it.
#[cfg(windows)]
fn git_bash() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| PathBuf::from(dir).join("Git").join("bin").join("bash.exe"))
        .find(|bash| bash.is_file())
}

#[cfg(windows)]
#[test]
fn under_git_bash_the_script_hands_over_to_its_powershell_form() -> TestResult {
    let Some(bash) = git_bash() else {
        eprintln!("SKIPPED: no Git for Windows bash");
        return Ok(());
    };
    let root = Root::new()?;
    // the forms an agent's Git Bash passes: a POSIX path to the script, a relative one to the image
    let script = root.path.join("scripts").join("show-image.sh");
    let script = script.to_str().ok_or("a UTF-8 path")?.replace('\\', "/");
    let output = root.run(&bash, &[&script, "shot.png"], true)?;
    assert!(output.status.success(), "show-image.sh failed: {}", stderr(&output));
    assert_page_opened(&root, "60")
}

#[test]
fn without_an_image_it_prints_its_usage_and_exits_2() -> TestResult {
    let root = Root::new()?;
    let output = show_image(&root, &[], true)?;
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("usage: show-image."), "{}", stderr(&output));
    assert!(root.calls().is_empty());
    Ok(())
}

#[test]
fn a_missing_image_is_refused() -> TestResult {
    let root = Root::new()?;
    let output = show_image(&root, &["missing.png"], true)?;
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no such file: missing.png"),
        "{}",
        stderr(&output)
    );
    assert!(root.calls().is_empty());
    Ok(())
}

#[test]
fn outside_ngterm_it_opens_nothing() -> TestResult {
    let root = Root::new()?;
    let output = show_image(&root, &["shot.png"], false)?;
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("not inside ngterm"), "{}", stderr(&output));
    assert!(root.calls().is_empty());
    Ok(())
}
