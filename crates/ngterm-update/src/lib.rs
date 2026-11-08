//! Release updates are opt-in at packaging time. Developer and package-manager builds have no marker.
//! Downloads are verified before extraction; installation replaces a complete tree with rollback on failure.
//! App releases come from GitHub alone ([`Source::github`]); [`fakes`] (feature `fakes`) serves tests a local one.
//! [`web_engine`] holds the pinned, verified Linux engine runtime download and per-user cache, with a URL-only
//! mirror override and atomic runtime publication.

mod download;
#[cfg(any(test, feature = "fakes"))]
pub mod fakes;
mod install;
mod staging;
pub mod web_engine;

use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;

pub use download::{Release, Source};
pub use install::Prepared;

/// The release signing keys this build trusts. A build trusts exactly what its source commit's `trust/` directory
/// says, so no release can talk an installed build out of checking.
#[derive(Debug, Clone, Default)]
pub struct Trust {
    /// The key `SHA256SUMS.minisig` must verify against; none (an empty `trust/minisign.pub`) skips the signature.
    minisign: Option<minisign_verify::PublicKey>,
}

impl Trust {
    /// What release builds trust: the committed `trust/minisign.pub`.
    ///
    /// # Errors
    /// Returns an error when the committed key doesn't parse, which a test rules out for every commit.
    pub fn release() -> Result<Self, Error> {
        Self::from_files(include_str!("../trust/minisign.pub"))
    }

    /// Trust from a `minisign.pub` file's contents; empty or whitespace means no key.
    pub(crate) fn from_files(minisign: &str) -> Result<Self, Error> {
        let minisign = if minisign.trim().is_empty() {
            None
        } else {
            Some(
                minisign_verify::PublicKey::decode(minisign.trim_start())
                    .map_err(|e| error(format!("The release signing key is invalid: {e}")))?,
            )
        };
        Ok(Self { minisign })
    }
}

/// The release archive/MSI marker, relative to the prefix above `bin` (`usr/` for Linux archives).
/// macOS app bundles instead carry `Contents/Resources/update-format`.
pub const MARKER: &str = "share/ngterm/update-format";
/// Release notes and the manual download fallback.
pub const RELEASES_URL: &str = "https://github.com/nickalie/ngterm/releases";

/// An updater failure, safe to show in the UI; no request headers or environment values are included.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] ureq::Error),
}

fn error(message: impl Into<String>) -> Error {
    Error::Message(message.into())
}

/// The release asset matching the installation, rather than merely the host OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    LinuxArchive,
    AppImage,
    MacArchive,
    MacBundle,
    WindowsArchive,
    WindowsMsi,
}

impl Format {
    fn asset(self, version: &Version, arch: &str) -> Result<String, Error> {
        let (platform, arch, extension) = match (self, arch) {
            (Self::LinuxArchive, "x86_64" | "aarch64") => ("linux", arch, "tar.gz"),
            (Self::AppImage, "x86_64" | "aarch64") => ("linux", arch, "AppImage"),
            (Self::MacArchive, "x86_64") => ("macos", arch, "tar.gz"),
            (Self::MacArchive, "aarch64") => ("macos", "arm64", "tar.gz"),
            (Self::MacBundle, "x86_64") => ("macos", arch, "app.zip"),
            (Self::MacBundle, "aarch64") => ("macos", "arm64", "app.zip"),
            (Self::WindowsArchive, "x86_64") => ("windows", arch, "zip"),
            (Self::WindowsMsi, "x86_64") => ("windows", arch, "msi"),
            _ => return Err(error("No release binary is available for this architecture.")),
        };
        Ok(format!("ngterm-{version}-{platform}-{arch}.{extension}"))
    }

    fn from_marker(marker: &str) -> Option<Self> {
        match marker.trim() {
            "linux-archive" if cfg!(target_os = "linux") => Some(Self::LinuxArchive),
            "macos-archive" if cfg!(target_os = "macos") => Some(Self::MacArchive),
            "macos-bundle" if cfg!(target_os = "macos") => Some(Self::MacBundle),
            "windows-archive" if cfg!(windows) => Some(Self::WindowsArchive),
            "windows-msi" if cfg!(windows) => Some(Self::WindowsMsi),
            _ => None,
        }
    }
}

/// A complete replaceable installation; application images use their original file outside the mounted filesystem.
#[derive(Debug, Clone)]
pub struct Installation {
    root: PathBuf,
    executable: PathBuf,
    format: Format,
    image_identity: Option<Vec<u8>>,
}

impl Installation {
    /// Whether a state or configuration directory would be replaced with this installation. Existing symlinks are
    /// resolved, so callers can refuse updates that would remove user data stored inside a portable installation.
    /// Unresolvable paths are conservatively treated as protected.
    #[must_use]
    pub fn contains_path(&self, path: &Path) -> bool {
        resolved_path(path)
            .and_then(|path| Ok(path.starts_with(resolved_path(&self.root)?)))
            .unwrap_or(true)
    }

    /// Refuses installation when any current state/configuration path is inside it or cannot be resolved safely.
    ///
    /// # Errors
    /// Returns an explanation when replacing the installation could remove user data.
    pub fn validate_paths(&self, paths: &[PathBuf]) -> Result<(), Error> {
        self.validate_access()?;
        if paths.iter().any(|path| self.contains_path(path)) {
            return Err(error(
                "Automatic updates would replace your state or configuration directory, or its location could not be resolved safely. Move those directories outside the ngterm installation, or update manually.",
            ));
        }
        Ok(())
    }

    /// Removes abandoned stages for this installation, preserving live helpers and recovery backups.
    ///
    /// # Errors
    /// Returns an error if the installation's parent cannot be read. Busy stages are retained for a later attempt.
    pub fn cleanup_stale(&self) -> Result<usize, Error> {
        staging::cleanup(self)
    }

    fn validate_access(&self) -> Result<(), Error> {
        let metadata = fs::metadata(&self.root)?;
        if !(if self.format == Format::AppImage {
            metadata.is_file()
        } else {
            metadata.is_dir()
        }) {
            return Err(error("The installation path no longer has the expected file type."));
        }
        #[cfg(target_os = "macos")]
        if self.format == Format::MacBundle {
            // SAFETY: geteuid only reads the current process's effective user ID.
            bundle_access(&self.root, unsafe { libc::geteuid() })?;
        }
        Ok(())
    }

    /// Finds only installations explicitly enabled by release packaging. Paths are canonical to preserve launcher
    /// symlinks. An application image environment is accepted only when this executable is inside its mounted directory.
    ///
    /// # Errors
    /// Returns an explanation when this installation is not enabled for automatic updates or its path is invalid.
    pub fn detect() -> Result<Self, Error> {
        let executable = dunce::canonicalize(executable_on_disk(std::env::current_exe()?))?;
        if cfg!(target_os = "linux")
            && let (Some(image), Some(appdir)) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"))
            && executable.starts_with(Path::new(&appdir))
        {
            let root = fs::canonicalize(image)?;
            if root.is_file() {
                return Ok(Self {
                    image_identity: Some(crate::install::image_stamp(&root)?),
                    executable: root.clone(),
                    root,
                    format: Format::AppImage,
                });
            }
        }
        Self::at(&executable)
    }

    fn at(executable: &Path) -> Result<Self, Error> {
        let executable = dunce::canonicalize(executable)?;
        let holder = executable
            .parent()
            .ok_or_else(|| error("Executable has no directory."))?;
        let root = holder
            .parent()
            .ok_or_else(|| error("Executable has no install root."))?;
        let (root, marker) = if cfg!(target_os = "macos") && holder.ends_with("Contents/MacOS") {
            (
                root.parent().ok_or_else(|| error("Invalid app bundle."))?,
                root.join("Resources/update-format"),
            )
        } else {
            (root, root.join(MARKER))
        };
        let format = fs::read_to_string(marker).ok().and_then(|text| Format::from_marker(&text))
            .ok_or_else(|| error("Automatic updates are available for official release archives, AppImages, macOS app bundles and Windows installers. Update package-manager installations with your package manager; rebuild development installations from source."))?;
        let root = if format == Format::LinuxArchive {
            root.parent()
                .filter(|_| root.file_name().is_some_and(|name| name == "usr"))
                .ok_or_else(|| error("Invalid Linux archive layout."))?
        } else {
            root
        };
        if format == Format::MacBundle && root.extension().is_none_or(|extension| extension != "app") {
            return Err(error(
                "Keep the macOS release inside its .app bundle to use automatic updates.",
            ));
        }
        if matches!(
            format,
            Format::LinuxArchive | Format::MacArchive | Format::WindowsArchive
        ) && root.file_name().is_none_or(|name| name != "ngterm")
        {
            return Err(error(
                "For automatic updates, extract the release archive into a dedicated directory named ngterm. Shared installation prefixes must be updated manually.",
            ));
        }
        let installation = Self {
            root: root.to_path_buf(),
            executable,
            format,
            image_identity: None,
        };
        installation.validate_access()?;
        Ok(installation)
    }
}

#[cfg(all(unix, any(target_os = "macos", test)))]
fn bundle_access(root: &Path, user: u32) -> Result<(), Error> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(root)?;
    if metadata.uid() != user || (metadata.is_dir() && metadata.permissions().mode() & 0o700 != 0o700) {
        return Err(error(
            "This macOS app bundle is not owned and writable by your user. Update it manually with the PKG or DMG.",
        ));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(root)? {
            bundle_access(&entry?.path(), user)?;
        }
    }
    Ok(())
}

// Resolve missing leaf directories through their nearest existing ancestor, including ancestor symlinks. Refuse
// unreadable paths rather than treating them as external; caller-created state can appear there before installation.
fn resolved_path(path: &Path) -> std::io::Result<PathBuf> {
    match dunce::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let absolute = std::path::absolute(path)?;
            let name = absolute.file_name().ok_or(error)?;
            let parent = absolute
                .parent()
                .ok_or_else(|| std::io::Error::other("Path has no parent"))?;
            Ok(resolved_path(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

impl Installation {
    fn identity_file(&self) -> PathBuf {
        let prefix = match self.format {
            Format::LinuxArchive => self.root.join("usr"),
            Format::MacBundle => self.root.join("Contents"),
            _ => self.root.clone(),
        };
        prefix.join("share/ngterm/app-BUILD")
    }

    fn installed_version(&self) -> Option<Version> {
        let identity = fs::read_to_string(self.identity_file()).ok()?;
        Version::parse(identity.lines().next()?).ok()
    }
}

#[cfg(unix)]
fn executable_on_disk(executable: PathBuf) -> PathBuf {
    use std::os::unix::ffi::OsStrExt as _;
    if !executable.exists()
        && let Some(path) = executable.as_os_str().as_bytes().strip_suffix(b" (deleted)")
    {
        return PathBuf::from(std::ffi::OsStr::from_bytes(path));
    }
    executable
}

#[cfg(not(unix))]
fn executable_on_disk(executable: PathBuf) -> PathBuf {
    executable
}

#[cfg(test)]
mod tests;
