//! What the core needs to start hooks and custom commands: the frontend's [`ProcessLauncher`], a way back onto the
//! main loop for their exits, the socket they are told about, and the `PATH` they run with (upstream's
//! `CommandPath.swift`, the socket provider the runners take).

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ngterm_host::ProcessLauncher;

use crate::Core;
use crate::paths::ConfigLocator;

/// A step to run on the main loop with the core.
pub type CoreTask = Box<dyn FnOnce(&mut Core) + Send + 'static>;

/// Queues a [`CoreTask`] on the main loop, which settles after it as after any mutation.
pub type CorePoster = Arc<dyn Fn(CoreTask) + Send + Sync>;

/// The frontend's process seam, installed once with [`Core::set_process_host`].
#[derive(Clone)]
pub struct ProcessHost {
    pub launcher: Arc<dyn ProcessLauncher>,
    pub poster: CorePoster,
    /// What children get as `AGT_SOCKET`: the owned socket, or `<socket>.unavailable`.
    pub socket: String,
    /// The directory of the `ngtermctl` beside the app binary, put first on `PATH`.
    pub cli_dir: Option<PathBuf>,
    /// The app's own `PATH`, widened for children.
    pub path: Option<String>,
}

impl fmt::Debug for ProcessHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessHost")
            .field("socket", &self.socket)
            .field("cli_dir", &self.cli_dir)
            .finish_non_exhaustive()
    }
}

/// The process seam and where the config files live.
#[derive(Debug, Default)]
pub(crate) struct ProcessRuntime {
    pub(crate) host: Option<ProcessHost>,
    pub(crate) config: ConfigLocator,
}

/// The `PATH` used when the app's environment carries none: the system directories a login shell starts from.
#[cfg(unix)]
fn default_path() -> OsString {
    "/usr/local/bin:/usr/bin:/bin".into()
}

/// The `PATH` used when the app's environment carries none: the system part of Windows' own default `Path`
/// (`%SystemRoot%\system32;%SystemRoot%;…\Wbem;…\WindowsPowerShell\v1.0\;…\OpenSSH\`), `SystemRoot` expanded.
#[cfg(windows)]
fn default_path() -> OsString {
    let root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let dirs = [
        root.join("System32"),
        root.clone(),
        root.join(r"System32\Wbem"),
        root.join(r"System32\WindowsPowerShell\v1.0\"),
        root.join(r"System32\OpenSSH\"),
    ];
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Where a user-installed `ngtermctl` lands when it is not on the app's own `PATH`: `~/.local/bin`, the install's link
/// (upstream appends its installer's target for the same reason: an app started from the desktop may lack it). Windows
/// has none: the install puts the bundled tool's own folder on the user's `PATH`, which leads the widened one anyway,
/// and an app started from Explorer inherits the user's whole `PATH`.
fn user_bin_dir(home: &Path) -> Option<PathBuf> {
    (!cfg!(windows)).then(|| home.join(".local/bin"))
}

/// Whether two `PATH` entries name the same directory: byte for byte on Unix; on Windows without case and a trailing
/// separator, as Windows looks folders up.
#[cfg(unix)]
fn same_entry(left: &Path, right: &Path) -> bool {
    left.as_os_str() == right.as_os_str()
}

#[cfg(windows)]
fn same_entry(left: &Path, right: &Path) -> bool {
    let fold = |entry: &Path| {
        entry
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    fold(left) == fold(right)
}

/// The `PATH` a hook or custom command runs with (upstream's `CommandPath.widened`): the bundled CLI's directory first,
/// so a bare `ngtermctl` reaches this build, then the app's own entries (the platform's default when it has none), then
/// the user's tool directory when absent ([`user_bin_dir`]); duplicates and empty entries dropped. Entries are split and
/// joined by the platform's rules (`:` on Unix; `;` on Windows, a quoted entry kept whole).
#[must_use]
pub fn widened_path(path: Option<&str>, cli_dir: Option<&Path>, home: Option<&Path>) -> String {
    let base = path
        .filter(|path| !path.is_empty())
        .map_or_else(default_path, OsString::from);
    let mut entries: Vec<PathBuf> = Vec::new();
    let mut add = |dir: PathBuf| {
        let joinable = std::env::join_paths([&dir]).is_ok();
        if !dir.as_os_str().is_empty() && joinable && !entries.iter().any(|entry| same_entry(entry, &dir)) {
            entries.push(dir);
        }
    };
    if let Some(dir) = cli_dir {
        add(dir.to_path_buf());
    }
    std::env::split_paths(&base).for_each(&mut add);
    if let Some(dir) = home.and_then(user_bin_dir) {
        add(dir);
    }
    std::env::join_paths(entries)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

impl Core {
    /// Installs the frontend's process seam; until then hooks and custom commands fail to start.
    pub fn set_process_host(&mut self, host: ProcessHost) {
        self.processes.host = Some(host);
    }

    /// Captures where `keymap.conf`, `hooks.conf` and `remotes.conf` live (with the `configDirectory` setting on top).
    pub fn set_config_locator(&mut self, locator: ConfigLocator) {
        self.processes.config = locator;
        self.repoint_remotes();
    }

    /// The config directory: the `configDirectory` setting, else the environment's default.
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        let setting = self.settings().config_directory.as_deref().map(Path::new);
        self.processes.config.config_dir(setting)
    }

    /// The `PATH` children run with.
    pub(crate) fn child_path(&self) -> String {
        let host = self.processes.host.as_ref();
        widened_path(
            host.and_then(|host| host.path.as_deref()),
            host.and_then(|host| host.cli_dir.as_deref()),
            self.processes.config.home(),
        )
    }

    /// What children get as `AGT_SOCKET`.
    pub(crate) fn child_socket(&self) -> String {
        self.processes
            .host
            .as_ref()
            .map(|host| host.socket.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn widening_puts_the_bundled_cli_first_and_appends_the_user_bin() {
        assert_eq!(
            widened_path(
                Some("/usr/bin:/bin:/usr/bin"),
                Some(Path::new("/opt/ngterm/bin")),
                Some(Path::new("/home/u"))
            ),
            "/opt/ngterm/bin:/usr/bin:/bin:/home/u/.local/bin"
        );
        assert_eq!(
            widened_path(Some("/home/u/.local/bin:/bin"), None, Some(Path::new("/home/u"))),
            "/home/u/.local/bin:/bin"
        );
        // empty entries dropped, the bundled CLI already on the PATH kept first once
        assert_eq!(
            widened_path(
                Some("/usr/bin::/opt/ngterm/bin:"),
                Some(Path::new("/opt/ngterm/bin")),
                None
            ),
            "/opt/ngterm/bin:/usr/bin"
        );
        // a directory with the separator in its name cannot be an entry
        assert_eq!(widened_path(Some("/bin"), Some(Path::new("/opt/a:b")), None), "/bin");
    }

    #[test]
    #[cfg(windows)]
    fn widening_splits_and_joins_by_windows_rules() {
        // drive letters stay whole; the bundled CLI goes first, a copy differing in case or a trailing separator is
        // dropped, and nothing is appended for the home
        assert_eq!(
            widened_path(
                Some(r"C:\Windows\System32;C:\Program Files\Git\cmd;c:\windows\system32\;;D:\tools"),
                Some(Path::new(r"C:\Program Files\ngterm")),
                Some(Path::new(r"C:\Users\u"))
            ),
            r"C:\Program Files\ngterm;C:\Windows\System32;C:\Program Files\Git\cmd;D:\tools"
        );
        // the bundled CLI already on the PATH in another case
        assert_eq!(
            widened_path(
                Some(r"c:\program files\NGTERM;C:\Windows"),
                Some(Path::new(r"C:\Program Files\ngterm")),
                None
            ),
            r"C:\Program Files\ngterm;C:\Windows"
        );
        // a quoted entry holding the separator is one entry, quoted again on the way out
        assert_eq!(
            widened_path(Some(r#""C:\a;b";C:\Windows"#), None, None),
            r#""C:\a;b";C:\Windows"#
        );
    }

    #[test]
    fn widening_falls_back_to_the_platform_default() {
        let default = default_path().into_string().unwrap();
        assert!(!default.is_empty());
        assert_eq!(widened_path(Some(""), None, None), default);
        assert_eq!(widened_path(None, None, None), default);
        let first = std::env::split_paths(&default).next().unwrap();
        if cfg!(windows) {
            assert!(first.ends_with("System32"), "{default}");
        } else {
            assert_eq!(first, Path::new("/usr/local/bin"));
        }
    }
}
