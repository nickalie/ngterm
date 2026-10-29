//! Where state lives: the state directory and the control socket, in upstream's precedence
//! (`ControlServer.defaultSocketPath` in `agterm/Control/ControlServer.swift`: `AGTERM_CONTROL_SOCKET`, else
//! `ControlResolve.socketPath` over `AGTERM_STATE_DIR` or `PersistenceStore.defaultDirectory`), under this build's
//! names (`NGTERM_CONTROL_SOCKET`, `NGTERM_STATE_DIR`).
//!
//! The Linux base: `PersistenceStore.defaultDirectory` is Foundation's application support directory, which on Linux
//! is the XDG data home of the XDG Base Directory spec (`$XDG_DATA_HOME`, else `~/.local/share`). An empty
//! `NGTERM_STATE_DIR` counts as unset, as the spec treats an empty variable; upstream would take an empty value as
//! given. The directory name is this build's own (`ngterm`), not upstream's `agterm`.
//!
//! The Windows bases are the user's known folders (`SHGetKnownFolderPath`), never the XDG variables: the state
//! directory is `FOLDERID_LocalAppData\ngterm` (per machine, as Foundation's application support directory is), the
//! config directory `FOLDERID_RoamingAppData\ngterm` (it roams with the profile, as a user's settings do) and the
//! home `FOLDERID_Profile` when `HOME` is unset or empty. `NGTERM_STATE_DIR` and `NGTERM_CONTROL_SOCKET` still come
//! first, and without a known folder the base is `%TEMP%\ngterm`, as `/tmp/ngterm` is on Unix.
//!
//! On Windows the control socket's path names a named pipe instead of a file ([`control_pipe_name`]): the path stays
//! the identity the app, `ngtermctl` and `NGTERM_SOCKET` share, and the pipe's name is derived from it.
//!
//! Every function reads the environment through [`Env`], so tests pass a [`MapEnv`] instead of mutating the
//! process environment.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;

/// Overrides the whole state directory (test isolation, `run-gpui-dev.sh`).
pub const STATE_DIR_ENV: &str = "NGTERM_STATE_DIR";
/// Overrides the app's control socket path. Only the app reads it: `ngtermctl` resolves through the state dir.
pub const CONTROL_SOCKET_ENV: &str = "NGTERM_CONTROL_SOCKET";
/// The control socket's file name inside the state directory.
pub const SOCKET_FILE_NAME: &str = "ngterm.sock";
/// The state directory's name under the XDG data home (the local application data on Windows), the config
/// directory's under `~/.config` (the roaming application data on Windows) and the HUD bodies' under the runtime dir.
pub const APP_DIR_NAME: &str = "ngterm";
/// The Unix base used when no home directory can be found, as upstream's `agtermctl` (`/tmp/agterm`) under this
/// build's state directory name; Windows' is under its temporary directory ([`fallback_dir`]).
pub const FALLBACK_STATE_DIR: &str = "/tmp/ngterm";

/// A Windows known folder ([`Env::known_folder`]), a base Windows keeps a user's files under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    /// `FOLDERID_Profile`: the user's profile directory, the home.
    Profile,
    /// `FOLDERID_LocalAppData`: the user's application data on this machine, the state directory's base.
    LocalAppData,
    /// `FOLDERID_RoamingAppData`: the user's application data that roams with the profile, the config directory's
    /// base.
    RoamingAppData,
}

/// The process environment as the resolvers see it.
pub trait Env {
    /// The variable `key`, unset or not.
    fn var(&self, key: &str) -> Option<OsString>;
    /// The home directory from the password database, the fallback when `HOME` is unset or empty (on Windows,
    /// after the profile's known folder: `USERPROFILE`, else the profile directory the system reports).
    fn passwd_home(&self) -> Option<PathBuf>;
    /// The current user's known folder `folder` on Windows; `None` on other OSes, or when it does not resolve.
    fn known_folder(&self, folder: KnownFolder) -> Option<PathBuf>;
    /// The names of every variable set, in no particular order.
    fn names(&self) -> Vec<OsString>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }

    fn passwd_home(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }

    #[cfg(windows)]
    fn known_folder(&self, folder: KnownFolder) -> Option<PathBuf> {
        windows::known_folder(folder)
    }

    #[cfg(not(windows))]
    fn known_folder(&self, _folder: KnownFolder) -> Option<PathBuf> {
        None
    }

    fn names(&self) -> Vec<OsString> {
        std::env::vars_os().map(|(name, _)| name).collect()
    }
}

/// A fixed environment for tests.
#[derive(Debug, Clone, Default)]
pub struct MapEnv {
    vars: HashMap<String, OsString>,
    passwd_home: Option<PathBuf>,
    known_folders: HashMap<KnownFolder, PathBuf>,
}

impl MapEnv {
    /// An empty environment with no password-database home and no known folder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `key` to `value`.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<OsString>) -> Self {
        self.vars.insert(key.to_owned(), value.into());
        self
    }

    /// Sets the password-database home.
    #[must_use]
    pub fn with_passwd_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.passwd_home = Some(home.into());
        self
    }

    /// Sets the known folder `folder`, which the resolvers read on Windows only.
    #[must_use]
    pub fn with_known_folder(mut self, folder: KnownFolder, path: impl Into<PathBuf>) -> Self {
        self.known_folders.insert(folder, path.into());
        self
    }
}

impl Env for MapEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        self.vars.get(key).cloned()
    }

    fn passwd_home(&self) -> Option<PathBuf> {
        self.passwd_home.clone()
    }

    fn known_folder(&self, folder: KnownFolder) -> Option<PathBuf> {
        self.known_folders.get(&folder).cloned()
    }

    fn names(&self) -> Vec<OsString> {
        self.vars.keys().map(OsString::from).collect()
    }
}

/// A variable that is set and non-empty.
fn non_empty(env: &dyn Env, key: &str) -> Option<OsString> {
    env.var(key).filter(|value| !value.is_empty())
}

/// The known folder `folder` on Windows; `None` elsewhere, whatever `env` holds.
fn windows_known_folder(env: &dyn Env, folder: KnownFolder) -> Option<PathBuf> {
    if cfg!(windows) { env.known_folder(folder) } else { None }
}

/// The user's home: `HOME` when set and non-empty, else (on Windows) the profile's known folder, else the password
/// database ([`Env::passwd_home`]). `HOME` stays first on Windows, where it is seldom set, as the explicit override
/// an isolated instance (the conformance harness) sets.
#[must_use]
pub fn home_dir(env: &dyn Env) -> Option<PathBuf> {
    non_empty(env, "HOME")
        .map(PathBuf::from)
        .or_else(|| windows_known_folder(env, KnownFolder::Profile))
        .or_else(|| env.passwd_home())
}

/// The XDG data home as Foundation reads it: `XDG_DATA_HOME` when it is an absolute path longer than `/`, else
/// `<home>/.local/share`. The Unix state base; Windows reads its known folders instead.
#[must_use]
pub fn xdg_data_home(env: &dyn Env) -> Option<PathBuf> {
    let explicit = env
        .var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.as_os_str().len() > 1);
    explicit.or_else(|| home_dir(env).map(|home| home.join(".local/share")))
}

/// The base when no home (on Windows, no known folder) resolves: [`FALLBACK_STATE_DIR`] on Unix; on Windows
/// `<TEMP>\ngterm`, `TEMP` when set and non-empty, else the system's temporary directory.
#[must_use]
pub fn fallback_dir(env: &dyn Env) -> PathBuf {
    if cfg!(windows) {
        non_empty(env, "TEMP")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(APP_DIR_NAME)
    } else {
        PathBuf::from(FALLBACK_STATE_DIR)
    }
}

/// The state directory with no override: `<XDG data home>/ngterm` on Unix, `<local application data>\ngterm` on
/// Windows, or [`fallback_dir`] without either.
#[must_use]
pub fn default_state_dir(env: &dyn Env) -> PathBuf {
    let base = if cfg!(windows) {
        windows_known_folder(env, KnownFolder::LocalAppData)
    } else {
        xdg_data_home(env)
    };
    base.map_or_else(|| fallback_dir(env), |base| base.join(APP_DIR_NAME))
}

/// The state directory: a non-empty `NGTERM_STATE_DIR`, else [`default_state_dir`].
#[must_use]
pub fn state_dir(env: &dyn Env) -> PathBuf {
    non_empty(env, STATE_DIR_ENV).map_or_else(|| default_state_dir(env), PathBuf::from)
}

/// The control socket inside `state_dir`; the rendezvous the app and `ngtermctl` share.
#[must_use]
pub fn socket_in(state_dir: &Path) -> PathBuf {
    state_dir.join(SOCKET_FILE_NAME)
}

/// The socket the app binds: a non-empty `NGTERM_CONTROL_SOCKET`, else `<state dir>/ngterm.sock`.
#[must_use]
pub fn app_socket_path(env: &dyn Env) -> PathBuf {
    non_empty(env, CONTROL_SOCKET_ENV).map_or_else(|| socket_in(&state_dir(env)), PathBuf::from)
}

/// The directory of the user-editable config files (`keymap.conf`, `hooks.conf`, the ngterm-scoped `ghostty.conf`),
/// as upstream's `ConfigPaths.configDirectory`: a non-empty `setting` (`AppSettings.configDirectory`), else
/// `<NGTERM_STATE_DIR>/config` when the state dir is overridden, else `<home>/.config/ngterm` on Unix and
/// `<roaming application data>\ngterm` on Windows, else `<fallback dir>/config` ([`fallback_dir`]).
#[must_use]
pub fn config_dir(env: &dyn Env, setting: Option<&Path>) -> PathBuf {
    if let Some(setting) = setting.filter(|path| !path.as_os_str().is_empty()) {
        return setting.to_path_buf();
    }
    if let Some(state) = non_empty(env, STATE_DIR_ENV) {
        return PathBuf::from(state).join("config");
    }
    let base = if cfg!(windows) {
        windows_known_folder(env, KnownFolder::RoamingAppData)
    } else {
        home_dir(env).map(|home| home.join(".config"))
    };
    base.map_or_else(|| fallback_dir(env).join("config"), |base| base.join(APP_DIR_NAME))
}

/// What [`config_dir`] reads of the environment, captured once so the core can resolve the config directory again
/// whenever the `configDirectory` setting changes. The default locator is an empty environment's, so its config
/// directory is the fallback one (`<fallback dir>/config`), never an empty, working-directory-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocator {
    /// [`config_dir`] without a setting: where the config lives unless the setting moves it.
    default_dir: PathBuf,
    home: Option<PathBuf>,
}

impl ConfigLocator {
    /// The locator of `env`.
    #[must_use]
    pub fn capture(env: &dyn Env) -> Self {
        Self {
            default_dir: config_dir(env, None),
            home: home_dir(env),
        }
    }

    /// [`config_dir`] under the captured environment.
    #[must_use]
    pub fn config_dir(&self, setting: Option<&Path>) -> PathBuf {
        match setting.filter(|path| !path.as_os_str().is_empty()) {
            Some(setting) => setting.to_path_buf(),
            None => self.default_dir.clone(),
        }
    }

    /// The captured home directory.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }
}

impl Default for ConfigLocator {
    fn default() -> Self {
        Self::capture(&MapEnv::new())
    }
}

/// Where the HUD body files go: a private per-user directory standing in for upstream's `NSTemporaryDirectory()`
/// (a per-user 0700 directory on macOS), never the shared `/tmp`. `<NGTERM_STATE_DIR>/hud` when the state dir is
/// overridden (instances stay apart, as [`config_dir`]), else on Unix `<XDG_RUNTIME_DIR>/ngterm/hud` when that is an
/// absolute path (the user's 0700 runtime directory, emptied at logout), else `<state dir>/hud`. Windows has no
/// runtime directory: its bodies go to `<state dir>\hud`, the state dir being the user's own local application data.
/// The writer makes the directory private: 0700 on Unix, an owner-only DACL on Windows (`owner_only`).
#[must_use]
pub fn hud_body_dir(env: &dyn Env) -> PathBuf {
    if let Some(state) = non_empty(env, STATE_DIR_ENV) {
        return PathBuf::from(state).join("hud");
    }
    let runtime = env
        .var("XDG_RUNTIME_DIR")
        .filter(|_| !cfg!(windows))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.as_os_str().len() > 1);
    runtime.map_or_else(
        || state_dir(env).join("hud"),
        |base| base.join(APP_DIR_NAME).join("hud"),
    )
}

/// The ngterm-scoped ghostty config inside a config directory: `<dir>/ghostty.conf`.
#[must_use]
pub fn ghostty_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("ghostty.conf")
}

/// The XDG config home Ghostty reads its user files under. On Unix an absolute `XDG_CONFIG_HOME`, else
/// `<home>/.config`. On Windows Ghostty's own choice at the pin (`src/os/xdg.zig`, `dir`): `XDG_CONFIG_HOME` whenever
/// it is set, even relative, else `LOCALAPPDATA`; the value chosen is used as given unless it is empty, and only then
/// does the home's `.config` take over (a set but empty `XDG_CONFIG_HOME` skips `LOCALAPPDATA`). The home there is
/// this build's [`home_dir`] (`HOME`, else the profile's known folder), where Ghostty's is `HOMEDRIVE` + `HOMEPATH`:
/// the two name the same profile on a stock account, and `HOME` keeps an isolated instance off the real one.
fn ghostty_config_home(env: &dyn Env) -> Option<PathBuf> {
    let explicit = if cfg!(windows) {
        env.var("XDG_CONFIG_HOME")
            .or_else(|| env.var("LOCALAPPDATA"))
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    } else {
        env.var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.as_os_str().len() > 1)
    };
    explicit.or_else(|| home_dir(env).map(|home| home.join(".config")))
}

/// The user's own ghostty directory, `<XDG config home>/ghostty` ([`ghostty_config_home`]): where the global config
/// files and the user's themes live. `None` without a home.
#[must_use]
pub fn global_ghostty_dir(env: &dyn Env) -> Option<PathBuf> {
    ghostty_config_home(env).map(|base| base.join("ghostty"))
}

/// The user's own ghostty config files, which the Settings toggle `inheritGlobalGhosttyConfig` loads below the
/// ngterm-scoped one, in Ghostty's load order at the pin (`Config.loadDefaultFiles`, `file_load.zig`): the legacy
/// `<ghostty dir>/config` (upstream reads `~/.config/ghostty/config`), then `<ghostty dir>/config.ghostty`, Ghostty's
/// name since 1.3; a missing one is skipped, and when both exist the second wins. Empty without a home.
#[must_use]
pub fn global_ghostty_config_paths(env: &dyn Env) -> Vec<PathBuf> {
    global_ghostty_dir(env)
        .map(|dir| vec![dir.join("config"), dir.join("config.ghostty")])
        .unwrap_or_default()
}

/// The ghostty config the Settings window writes: `<state dir>/ghostty-settings.conf`
/// (`GhosttyApp.settingsConfigURL`).
#[must_use]
pub fn ghostty_settings_config_path(env: &dyn Env) -> PathBuf {
    state_dir(env).join("ghostty-settings.conf")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(OsStr::new(suffix));
    PathBuf::from(raw)
}

/// The prefix of the control socket's pipe name on Windows ([`control_pipe_name`]).
pub const CONTROL_PIPE_PREFIX: &str = "ngterm-control-";

/// The Windows pipe name, without the `\\.\pipe\` prefix, that `path` stands for: `prefix` and the FNV-1a 64 hash
/// ([`crate::live::fnv1a64`]) of the path case-folded, with `/` as `\` and no trailing separator, so every spelling
/// Windows takes for one path names one pipe. The scheme ngterm-session's namespace pipe and the control socket's
/// pipe share. Defined on every OS, so a pipe can be named anywhere; `path` is taken as given (make it absolute
/// first, as [`control_pipe_name`] does).
#[must_use]
pub fn pipe_name(prefix: &str, path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = path.trim_end_matches('\\');
    format!("{prefix}{}", crate::live::fnv1a64(path.as_bytes()))
}

/// The pipe the control socket at `socket` is on Windows, without the `\\.\pipe\` prefix: [`pipe_name`] of
/// [`CONTROL_PIPE_PREFIX`] and the path made absolute against the current directory ([`std::path::absolute`]), so a
/// relative `NGTERM_CONTROL_SOCKET` or `--socket` names the same pipe as its absolute form. A path that cannot be made
/// absolute (empty) is taken as given. The one derivation the server and its clients share.
#[must_use]
pub fn control_pipe_name(socket: &Path) -> String {
    let absolute = std::path::absolute(socket).unwrap_or_else(|_| socket.to_path_buf());
    pipe_name(CONTROL_PIPE_PREFIX, &absolute)
}

/// `<socket>.lock`: the file whose exclusive lock (`flock`, `LockFileEx` on Windows) owns the socket path. It is never
/// removed: unlinking it would let the next instance lock a fresh inode and exclude nobody.
#[must_use]
pub fn ownership_lock_path(socket: &Path) -> PathBuf {
    with_suffix(socket, ".lock")
}

/// `<socket>.unavailable`: what a refused instance advertises as `NGTERM_SOCKET`, a path nothing creates, so its
/// shells cannot reach the instance that owns the socket.
#[must_use]
pub fn unavailable_socket_path(socket: &Path) -> PathBuf {
    with_suffix(socket, ".unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(path: &str) -> MapEnv {
        MapEnv::new().with("HOME", path)
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn hud_bodies_go_to_the_overridden_state_dir_else_the_runtime_dir_else_the_state_dir() {
        let runtime = home("/home/u").with("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(hud_body_dir(&runtime), Path::new("/run/user/1000/ngterm/hud"));
        let overridden = runtime.clone().with(STATE_DIR_ENV, "/tmp/inst");
        assert_eq!(hud_body_dir(&overridden), Path::new("/tmp/inst/hud"));
        for value in ["", "relative", "/"] {
            let env = home("/home/u").with("XDG_RUNTIME_DIR", value);
            assert_eq!(
                hud_body_dir(&env),
                Path::new("/home/u/.local/share/ngterm/hud"),
                "{value:?}"
            );
        }
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn the_global_ghostty_config_follows_the_xdg_config_home() {
        assert_eq!(
            global_ghostty_config_paths(&home("/home/u")),
            [
                Path::new("/home/u/.config/ghostty/config"),
                Path::new("/home/u/.config/ghostty/config.ghostty")
            ]
        );
        let env = home("/home/u").with("XDG_CONFIG_HOME", "/cfg");
        assert_eq!(global_ghostty_dir(&env).as_deref(), Some(Path::new("/cfg/ghostty")));
        let relative = home("/home/u").with("XDG_CONFIG_HOME", "rel");
        assert_eq!(
            global_ghostty_dir(&relative).as_deref(),
            Some(Path::new("/home/u/.config/ghostty"))
        );
        let local = home("/home/u").with("LOCALAPPDATA", "/local");
        assert_eq!(
            global_ghostty_dir(&local).as_deref(),
            Some(Path::new("/home/u/.config/ghostty")),
            "LOCALAPPDATA is Windows' alone"
        );
    }

    /// Ghostty's `loadDefaultFiles` order on every OS: the legacy `config`, then `config.ghostty`, in one directory.
    #[test]
    fn the_global_ghostty_config_files_are_the_legacy_one_then_config_ghostty() {
        let env = MapEnv::new().with("HOME", std::env::temp_dir().as_os_str());
        let dir = global_ghostty_dir(&env).expect("a home");
        assert_eq!(dir.file_name(), Some(OsStr::new("ghostty")));
        assert_eq!(
            global_ghostty_config_paths(&env),
            [dir.join("config"), dir.join("config.ghostty")]
        );
        assert_eq!(global_ghostty_config_paths(&MapEnv::new()), Vec::<PathBuf>::new());
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn xdg_data_home_unset_falls_back_to_home() {
        assert_eq!(state_dir(&home("/home/u")), Path::new("/home/u/.local/share/ngterm"));
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn xdg_data_home_set_is_used() {
        let env = home("/home/u").with("XDG_DATA_HOME", "/data");
        assert_eq!(state_dir(&env), Path::new("/data/ngterm"));
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn invalid_xdg_data_home_is_ignored() {
        for value in ["", "relative/data", "/"] {
            let env = home("/home/u").with("XDG_DATA_HOME", value);
            assert_eq!(state_dir(&env), Path::new("/home/u/.local/share/ngterm"), "{value:?}");
        }
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn missing_home_uses_the_password_database_then_tmp() {
        let env = MapEnv::new().with("HOME", "").with_passwd_home("/var/home/u");
        assert_eq!(state_dir(&env), Path::new("/var/home/u/.local/share/ngterm"));
        assert_eq!(state_dir(&MapEnv::new()), Path::new(FALLBACK_STATE_DIR));
        assert_eq!(
            fallback_dir(&MapEnv::new().with("TEMP", "/t")),
            Path::new(FALLBACK_STATE_DIR)
        );
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn the_known_folders_are_windows_alone() {
        let env = MapEnv::new()
            .with_known_folder(KnownFolder::Profile, "/profile")
            .with_known_folder(KnownFolder::LocalAppData, "/local")
            .with_known_folder(KnownFolder::RoamingAppData, "/roaming");
        assert_eq!(home_dir(&env), None);
        assert_eq!(state_dir(&env), Path::new(FALLBACK_STATE_DIR));
        assert_eq!(config_dir(&env, None), Path::new("/tmp/ngterm/config"));
        assert_eq!(ProcessEnv.known_folder(KnownFolder::Profile), None);
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn state_dir_override_wins_and_empty_is_unset() {
        let env = home("/home/u").with("XDG_DATA_HOME", "/data");
        assert_eq!(
            state_dir(&env.clone().with(STATE_DIR_ENV, "/tmp/s")),
            Path::new("/tmp/s")
        );
        assert_eq!(state_dir(&env.with(STATE_DIR_ENV, "")), Path::new("/data/ngterm"));
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn socket_path_falls_back_to_the_state_dir() {
        let env = home("/home/u");
        assert_eq!(
            app_socket_path(&env),
            Path::new("/home/u/.local/share/ngterm/ngterm.sock")
        );
        let isolated = env.with(STATE_DIR_ENV, "/tmp/s");
        assert_eq!(app_socket_path(&isolated), Path::new("/tmp/s/ngterm.sock"));
        assert_eq!(
            app_socket_path(&isolated.clone().with(CONTROL_SOCKET_ENV, "")),
            Path::new("/tmp/s/ngterm.sock")
        );
        let explicit = isolated.with(CONTROL_SOCKET_ENV, "/tmp/x/ctl.sock");
        assert_eq!(app_socket_path(&explicit), Path::new("/tmp/x/ctl.sock"));
    }

    #[test]
    fn ownership_paths_append_to_the_socket_path() {
        let socket = Path::new("/tmp/s/ngterm.sock");
        assert_eq!(ownership_lock_path(socket), Path::new("/tmp/s/ngterm.sock.lock"));
        assert_eq!(
            unavailable_socket_path(socket),
            Path::new("/tmp/s/ngterm.sock.unavailable")
        );
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn config_dir_follows_upstream_precedence() {
        let env = home("/home/u").with("XDG_DATA_HOME", "/data");
        assert_eq!(config_dir(&env, None), Path::new("/home/u/.config/ngterm"));
        let isolated = env.clone().with(STATE_DIR_ENV, "/tmp/s");
        assert_eq!(config_dir(&isolated, None), Path::new("/tmp/s/config"));
        assert_eq!(config_dir(&isolated, Some(Path::new(""))), Path::new("/tmp/s/config"));
        assert_eq!(
            config_dir(&isolated, Some(Path::new("/etc/agt"))),
            Path::new("/etc/agt")
        );
        assert_eq!(
            config_dir(&env.with(STATE_DIR_ENV, ""), None),
            Path::new("/home/u/.config/ngterm")
        );
        assert_eq!(config_dir(&MapEnv::new(), None), Path::new("/tmp/ngterm/config"));
    }

    /// The locator resolves as [`config_dir`] over the environment it captured, on every OS.
    #[test]
    fn the_config_locator_resolves_as_the_environment_it_captured() {
        let state = std::env::temp_dir().join("ngterm-locator-state");
        let elsewhere = std::env::temp_dir().join("ngterm-locator-elsewhere");
        let env = MapEnv::new()
            .with("HOME", "home")
            .with(STATE_DIR_ENV, state.as_os_str())
            .with_known_folder(KnownFolder::RoamingAppData, "roaming");
        let locator = ConfigLocator::capture(&env);
        assert_eq!(locator.home(), Some(Path::new("home")));
        for setting in [None, Some(Path::new("")), Some(elsewhere.as_path())] {
            assert_eq!(locator.config_dir(setting), config_dir(&env, setting), "{setting:?}");
        }
        assert_eq!(locator.config_dir(None), state.join("config"));
        let plain = MapEnv::new()
            .with("HOME", "home")
            .with_known_folder(KnownFolder::RoamingAppData, "roaming");
        assert_eq!(
            ConfigLocator::capture(&plain).config_dir(None),
            config_dir(&plain, None)
        );
    }

    /// A core that never installs a locator still keeps its config files in the fallback directory, not the cwd.
    #[test]
    fn the_default_config_locator_is_an_empty_environments() {
        let locator = ConfigLocator::default();
        assert_eq!(locator, ConfigLocator::capture(&MapEnv::new()));
        assert_eq!(locator.config_dir(None), fallback_dir(&MapEnv::new()).join("config"));
        assert!(locator.config_dir(None).is_absolute());
        assert_eq!(locator.home(), None);
    }

    #[test]
    #[cfg_attr(windows, ignore = "the XDG bases, Unix's; the `windows_*` tests hold Windows' forms")]
    fn ghostty_config_files_sit_in_the_config_and_state_dirs() {
        assert_eq!(
            ghostty_config_path(Path::new("/tmp/s/config")),
            Path::new("/tmp/s/config/ghostty.conf")
        );
        let env = home("/home/u").with(STATE_DIR_ENV, "/tmp/s");
        assert_eq!(
            ghostty_settings_config_path(&env),
            Path::new("/tmp/s/ghostty-settings.conf")
        );
        assert_eq!(
            ghostty_settings_config_path(&home("/home/u")),
            Path::new("/home/u/.local/share/ngterm/ghostty-settings.conf")
        );
    }

    #[test]
    fn process_env_reads_the_real_environment() {
        assert_eq!(ProcessEnv.var("PATH"), std::env::var_os("PATH"));
    }

    /// A Windows user's known folders.
    #[cfg(windows)]
    fn windows_user() -> MapEnv {
        MapEnv::new()
            .with_known_folder(KnownFolder::Profile, r"C:\Users\u")
            .with_known_folder(KnownFolder::LocalAppData, r"C:\Users\u\AppData\Local")
            .with_known_folder(KnownFolder::RoamingAppData, r"C:\Users\u\AppData\Roaming")
    }

    #[cfg(windows)]
    #[test]
    fn windows_bases_are_the_known_folders_and_ignore_the_xdg_variables() {
        let plain = windows_user();
        let xdg = windows_user()
            .with("XDG_DATA_HOME", r"C:\xdg\data")
            .with("XDG_RUNTIME_DIR", r"C:\xdg\run");
        for env in [&plain, &xdg] {
            assert_eq!(home_dir(env).as_deref(), Some(Path::new(r"C:\Users\u")));
            assert_eq!(state_dir(env), Path::new(r"C:\Users\u\AppData\Local\ngterm"));
            assert_eq!(
                app_socket_path(env),
                Path::new(r"C:\Users\u\AppData\Local\ngterm\ngterm.sock")
            );
            assert_eq!(config_dir(env, None), Path::new(r"C:\Users\u\AppData\Roaming\ngterm"));
            assert_eq!(hud_body_dir(env), Path::new(r"C:\Users\u\AppData\Local\ngterm\hud"));
            assert_eq!(
                ghostty_settings_config_path(env),
                Path::new(r"C:\Users\u\AppData\Local\ngterm\ghostty-settings.conf")
            );
        }
        assert_eq!(
            config_dir(&plain, Some(Path::new(r"D:\cfg"))),
            Path::new(r"D:\cfg"),
            "the setting still wins"
        );
    }

    /// Ghostty's own lookup (`src/os/xdg.zig` at the pin): `XDG_CONFIG_HOME` whenever set, else `LOCALAPPDATA`; the
    /// home's `.config` only when the value chosen is empty.
    #[cfg(windows)]
    #[test]
    fn windows_global_ghostty_config_is_where_ghostty_looks() {
        assert_eq!(
            global_ghostty_config_paths(&windows_user()),
            [
                Path::new(r"C:\Users\u\.config\ghostty\config"),
                Path::new(r"C:\Users\u\.config\ghostty\config.ghostty")
            ]
        );
        let local = windows_user().with("LOCALAPPDATA", r"C:\Users\u\AppData\Local");
        assert_eq!(
            global_ghostty_dir(&local).as_deref(),
            Some(Path::new(r"C:\Users\u\AppData\Local\ghostty"))
        );
        assert_eq!(
            global_ghostty_dir(&local.clone().with("XDG_CONFIG_HOME", r"C:\cfg")).as_deref(),
            Some(Path::new(r"C:\cfg\ghostty"))
        );
        assert_eq!(
            global_ghostty_dir(&local.clone().with("XDG_CONFIG_HOME", "")).as_deref(),
            Some(Path::new(r"C:\Users\u\.config\ghostty")),
            "a set but empty XDG_CONFIG_HOME skips LOCALAPPDATA for the home"
        );
        assert_eq!(
            global_ghostty_dir(&local.clone().with("XDG_CONFIG_HOME", "relative")).as_deref(),
            Some(Path::new(r"relative\ghostty")),
            "a relative XDG_CONFIG_HOME is taken as given"
        );
        assert_eq!(
            global_ghostty_dir(&windows_user().with("LOCALAPPDATA", "")).as_deref(),
            Some(Path::new(r"C:\Users\u\.config\ghostty")),
            "an empty LOCALAPPDATA counts as unset"
        );
        assert_eq!(global_ghostty_dir(&MapEnv::new()), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_overrides_win_and_empty_ones_count_as_unset() {
        let isolated = windows_user().with(STATE_DIR_ENV, r"C:\inst");
        assert_eq!(state_dir(&isolated), Path::new(r"C:\inst"));
        assert_eq!(app_socket_path(&isolated), Path::new(r"C:\inst\ngterm.sock"));
        assert_eq!(config_dir(&isolated, None), Path::new(r"C:\inst\config"));
        assert_eq!(hud_body_dir(&isolated), Path::new(r"C:\inst\hud"));
        let explicit = isolated.with(CONTROL_SOCKET_ENV, r"C:\x\ctl.sock");
        assert_eq!(app_socket_path(&explicit), Path::new(r"C:\x\ctl.sock"));
        let empty = windows_user()
            .with(STATE_DIR_ENV, "")
            .with(CONTROL_SOCKET_ENV, "")
            .with("HOME", "");
        assert_eq!(state_dir(&empty), Path::new(r"C:\Users\u\AppData\Local\ngterm"));
        assert_eq!(
            app_socket_path(&empty),
            Path::new(r"C:\Users\u\AppData\Local\ngterm\ngterm.sock")
        );
        assert_eq!(
            config_dir(&empty, None),
            Path::new(r"C:\Users\u\AppData\Roaming\ngterm")
        );
        assert_eq!(home_dir(&empty).as_deref(), Some(Path::new(r"C:\Users\u")));
        let home = windows_user().with("HOME", r"D:\h").with_passwd_home(r"E:\p");
        assert_eq!(
            home_dir(&home).as_deref(),
            Some(Path::new(r"D:\h")),
            "HOME overrides the profile"
        );
        assert_eq!(
            config_dir(&home, None),
            Path::new(r"C:\Users\u\AppData\Roaming\ngterm"),
            "the config stays in the application data"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_without_known_folders_falls_back_to_the_temporary_directory() {
        let temp = MapEnv::new().with("TEMP", r"C:\t").with_passwd_home(r"C:\Users\p");
        assert_eq!(fallback_dir(&temp), Path::new(r"C:\t\ngterm"));
        assert_eq!(state_dir(&temp), Path::new(r"C:\t\ngterm"));
        assert_eq!(app_socket_path(&temp), Path::new(r"C:\t\ngterm\ngterm.sock"));
        assert_eq!(config_dir(&temp, None), Path::new(r"C:\t\ngterm\config"));
        assert_eq!(hud_body_dir(&temp), Path::new(r"C:\t\ngterm\hud"));
        assert_eq!(
            home_dir(&temp).as_deref(),
            Some(Path::new(r"C:\Users\p")),
            "no profile folder: the password database's stand-in"
        );
        let system = std::env::temp_dir().join(APP_DIR_NAME);
        assert_eq!(fallback_dir(&MapEnv::new()), system);
        assert_eq!(
            state_dir(&MapEnv::new().with("TEMP", "")),
            system,
            "an empty TEMP counts as unset"
        );
        assert_eq!(home_dir(&MapEnv::new()), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_env_reads_the_users_known_folders() {
        let folder = |folder| {
            let path = ProcessEnv.known_folder(folder).unwrap_or_else(|| panic!("{folder:?}"));
            assert!(path.is_absolute() && path.is_dir(), "{folder:?}: {}", path.display());
            path
        };
        let profile = folder(KnownFolder::Profile);
        let local = folder(KnownFolder::LocalAppData);
        let roaming = folder(KnownFolder::RoamingAppData);
        for (key, path) in [
            ("USERPROFILE", &profile),
            ("LOCALAPPDATA", &local),
            ("APPDATA", &roaming),
        ] {
            if let Some(value) = std::env::var_os(key).filter(|value| !value.is_empty()) {
                assert_eq!(&PathBuf::from(value), path, "{key}");
            }
        }
        assert_eq!(default_state_dir(&ProcessEnv), local.join(APP_DIR_NAME));
        if std::env::var_os(STATE_DIR_ENV).is_none() {
            assert_eq!(config_dir(&ProcessEnv, None), roaming.join(APP_DIR_NAME));
        }
    }

    #[test]
    fn a_pipe_name_ignores_case_separators_and_a_trailing_separator() {
        let name = pipe_name("p-", Path::new(r"C:\Users\Nick\AppData\Local\ngterm\ngterm.sock"));
        assert_eq!(
            name,
            pipe_name("p-", Path::new("c:/users/nick/appdata/local/ngterm/NGTERM.SOCK/"))
        );
        assert_eq!(
            name,
            format!(
                "p-{}",
                crate::live::fnv1a64(br"c:\users\nick\appdata\local\ngterm\ngterm.sock")
            )
        );
        assert_ne!(
            name,
            pipe_name("p-", Path::new(r"C:\Users\Nick\AppData\Local\ngterm\other.sock"))
        );
    }

    #[test]
    fn the_control_pipe_is_named_after_the_absolute_socket_path() {
        let current = std::env::current_dir().unwrap();
        let name = control_pipe_name(Path::new("ngterm.sock"));
        assert!(name.starts_with(CONTROL_PIPE_PREFIX), "{name}");
        assert_eq!(name.len(), CONTROL_PIPE_PREFIX.len() + 16);
        assert_eq!(name, pipe_name(CONTROL_PIPE_PREFIX, &current.join("ngterm.sock")));
        assert_eq!(name, control_pipe_name(&current.join("ngterm.sock")));
        // the refused instance's advertised path names a pipe of its own, which nothing serves
        assert_ne!(
            control_pipe_name(&unavailable_socket_path(&current.join("ngterm.sock"))),
            name
        );
        assert_eq!(
            control_pipe_name(Path::new("")),
            pipe_name(CONTROL_PIPE_PREFIX, Path::new(""))
        );
    }

    /// Every spelling Windows takes for one socket path names its one pipe.
    #[cfg(windows)]
    #[test]
    fn windows_every_spelling_of_a_socket_path_names_one_pipe() {
        let plain = control_pipe_name(Path::new(r"C:\a\ngterm\ngterm.sock"));
        for spelling in [
            r"C:\a\b\..\ngterm\ngterm.sock",
            r"C:\a\.\ngterm\ngterm.sock",
            "c:/A/ngterm/NGTERM.sock",
        ] {
            assert_eq!(control_pipe_name(Path::new(spelling)), plain, "{spelling}");
        }
    }
}
