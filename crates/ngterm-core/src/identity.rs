//! The installed build's identity, `version`'s `result.installed` (upstream's `AppIdentity.installed(bundleURL:)`,
//! #723): the version and commit of the build on disk where the running app was started from, read at request time.
//! A long-running app keeps running after its install is replaced underneath it (a package upgrade), so the two drift
//! apart; nothing caches the answer, so `version` reports the replacement as soon as it lands.
//!
//! Where it is read from, by the running executable's place ([`InstalledSource::of_executable`]):
//! - inside a macOS app bundle (`<name>.app/Contents/MacOS/ngterm`), the bundle's `Contents/Info.plist`
//!   (`CFBundleShortVersionString` and `GitCommit`), as upstream reads it;
//! - anywhere else, the staged layout's identity file, `share/ngterm/app-BUILD` of the directory above the one holding
//!   the executable (`bin/ngterm`, `bin\ngterm.exe` on Windows), written by `scripts/stage.sh` beside the binary: a
//!   version line, then a commit line ([`staged_text`]), as the staged binary itself prints them, so the file names
//!   the build beside it and never the staging shell's environment or a later checkout's version.
//!
//! Absent means unknown, never "same as the running build": a missing or malformed file, or one naming no version,
//! reads as `None`. Equal metadata does not prove equal executables: a rebuild from the same commit reads as equal.

use std::fs;
use std::path::{Path, PathBuf};

use ngterm_proto::AppIdentity;

/// The staged layout's identity file, relative to the install root (the directory above `bin/`).
pub const STAGED_FILE: &str = "share/ngterm/app-BUILD";

/// A bundle's property list, relative to the bundle.
const BUNDLE_INFO: &str = "Contents/Info.plist";
/// The bundle's version key, upstream's.
const VERSION_KEY: &str = "CFBundleShortVersionString";
/// The bundle's commit key, upstream's.
const COMMIT_KEY: &str = "GitCommit";

/// Where the installed build's metadata lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledSource {
    /// A staged layout's identity file ([`STAGED_FILE`]).
    Staged(PathBuf),
    /// A macOS app bundle's `Contents/Info.plist`.
    Bundle(PathBuf),
}

impl InstalledSource {
    /// The source for the running `executable`, as `current_exe` names it. Only its directories count, so the
    /// ` (deleted)` Linux appends to the file name of a binary replaced under the running process changes nothing.
    /// `None` for an executable with no directory above the one holding it.
    #[must_use]
    pub fn of_executable(executable: &Path) -> Option<Self> {
        let holder = executable.parent()?;
        if cfg!(target_os = "macos")
            && let Some(bundle) = bundle_of(holder)
        {
            return Some(Self::Bundle(bundle.join(BUNDLE_INFO)));
        }
        Some(Self::Staged(holder.parent()?.join(STAGED_FILE)))
    }

    /// The installed identity as the file holds it now; `None` when it cannot be read or names no version.
    #[must_use]
    pub fn read(&self) -> Option<AppIdentity> {
        match self {
            Self::Staged(file) => staged_identity(&fs::read_to_string(file).ok()?),
            Self::Bundle(plist) => bundle_identity(plist),
        }
    }
}

/// The bundle `holder` (an executable's directory) sits in: `<bundle>.app` for `<bundle>.app/Contents/MacOS`.
fn bundle_of(holder: &Path) -> Option<&Path> {
    let contents = holder
        .parent()
        .filter(|_| holder.file_name() == Some("MacOS".as_ref()))?;
    let bundle = contents
        .parent()
        .filter(|_| contents.file_name() == Some("Contents".as_ref()))?;
    bundle
        .extension()
        .is_some_and(|extension| extension == "app")
        .then_some(bundle)
}

/// The identity a staged file's `text` names: the first line's version, the second line's commit (dropped when absent,
/// empty or `unknown`); `None` without a version.
#[must_use]
pub fn staged_identity(text: &str) -> Option<AppIdentity> {
    let mut lines = text.lines().map(str::trim);
    let version = lines.next().filter(|version| !version.is_empty())?;
    Some(AppIdentity::from_recorded(version, lines.next()))
}

/// The staged file's text for `identity`, what [`staged_identity`] reads back: the version line, then the commit line
/// (empty when the build records none).
#[must_use]
pub fn staged_text(identity: &AppIdentity) -> String {
    format!(
        "{}\n{}\n",
        identity.version,
        identity.commit.as_deref().unwrap_or_default()
    )
}

/// The identity a bundle's property list at `plist` names, parsed from the file on every call.
fn bundle_identity(plist: &Path) -> Option<AppIdentity> {
    let info = plist::Value::from_file(plist).ok()?.into_dictionary()?;
    let version = info
        .get(VERSION_KEY)
        .and_then(plist::Value::as_string)
        .filter(|version| !version.is_empty())?;
    Some(AppIdentity::from_recorded(
        version,
        info.get(COMMIT_KEY).and_then(plist::Value::as_string),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(version: &str, commit: Option<&str>) -> AppIdentity {
        AppIdentity {
            version: version.to_owned(),
            commit: commit.map(str::to_owned),
        }
    }

    fn staged(dir: &Path) -> InstalledSource {
        InstalledSource::Staged(dir.join(STAGED_FILE))
    }

    fn write_staged(dir: &Path, text: &str) {
        let file = dir.join(STAGED_FILE);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    fn write_plist(bundle: &Path, entries: &[(&str, &str)]) {
        let info: plist::Dictionary = entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), plist::Value::String((*value).to_owned())))
            .collect();
        let file = bundle.join(BUNDLE_INFO);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        plist::Value::Dictionary(info).to_file_xml(file).unwrap();
    }

    #[test]
    fn installed_reads_version_and_commit_from_the_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        write_staged(dir.path(), "0.35.1\ndef5678\n");
        assert_eq!(staged(dir.path()).read(), Some(identity("0.35.1", Some("def5678"))));
    }

    #[test]
    fn staged_text_reads_back_as_the_identity_it_was_written_for() {
        for written in [identity("0.35.1", Some("def5678")), identity("0.35.1", None)] {
            let text = staged_text(&written);
            assert_eq!(staged_identity(&text), Some(written), "{text:?}");
        }
        assert_eq!(staged_text(&identity("0.35.1", None)), "0.35.1\n\n");
    }

    #[test]
    fn installed_reads_version_and_commit_from_the_bundle_plist() {
        let dir = tempfile::tempdir().unwrap();
        write_plist(dir.path(), &[(VERSION_KEY, "0.35.1"), (COMMIT_KEY, "def5678")]);
        let source = InstalledSource::Bundle(dir.path().join(BUNDLE_INFO));
        assert_eq!(source.read(), Some(identity("0.35.1", Some("def5678"))));
    }

    #[test]
    fn installed_keeps_the_version_when_the_commit_is_unusable() {
        for text in [
            "0.35.1",
            "0.35.1\n",
            "0.35.1\n\n",
            "0.35.1\nunknown\n",
            "0.35.1\r\n  \r\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_staged(dir.path(), text);
            assert_eq!(staged(dir.path()).read(), Some(identity("0.35.1", None)), "{text:?}");
        }
        for commit in [None, Some(""), Some("unknown")] {
            let dir = tempfile::tempdir().unwrap();
            let mut entries = vec![(VERSION_KEY, "0.35.1")];
            entries.extend(commit.map(|commit| (COMMIT_KEY, commit)));
            write_plist(dir.path(), &entries);
            let source = InstalledSource::Bundle(dir.path().join(BUNDLE_INFO));
            assert_eq!(source.read(), Some(identity("0.35.1", None)), "{commit:?}");
        }
    }

    #[test]
    fn installed_follows_the_file_as_it_changes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let source = staged(dir.path());
        write_staged(dir.path(), "0.35.1\ndef5678\n");
        assert_eq!(
            source.read().map(|installed| installed.version),
            Some("0.35.1".to_owned())
        );
        write_staged(dir.path(), "0.36.0\nabc1234\n");
        assert_eq!(source.read(), Some(identity("0.36.0", Some("abc1234"))));

        let bundle = InstalledSource::Bundle(dir.path().join(BUNDLE_INFO));
        write_plist(dir.path(), &[(VERSION_KEY, "0.35.1"), (COMMIT_KEY, "def5678")]);
        assert_eq!(
            bundle.read().map(|installed| installed.version),
            Some("0.35.1".to_owned())
        );
        write_plist(dir.path(), &[(VERSION_KEY, "0.36.0"), (COMMIT_KEY, "abc1234")]);
        assert_eq!(bundle.read(), Some(identity("0.36.0", Some("abc1234"))));
    }

    #[test]
    fn installed_is_none_without_a_readable_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(staged(dir.path()).read(), None, "missing");
        for text in ["", "\n", "  \ndef5678\n"] {
            write_staged(dir.path(), text);
            assert_eq!(staged(dir.path()).read(), None, "{text:?}");
        }
        fs::write(dir.path().join(STAGED_FILE), [0xff, 0xfe, b'\n']).unwrap();
        assert_eq!(staged(dir.path()).read(), None, "not UTF-8");

        let bundle = InstalledSource::Bundle(dir.path().join(BUNDLE_INFO));
        assert_eq!(bundle.read(), None, "missing");
        for entries in [&[(COMMIT_KEY, "def5678")][..], &[(VERSION_KEY, "")][..]] {
            write_plist(dir.path(), entries);
            assert_eq!(bundle.read(), None, "{entries:?}");
        }
        fs::write(dir.path().join(BUNDLE_INFO), "not a plist").unwrap();
        assert_eq!(bundle.read(), None, "malformed");
        plist::Value::Array(vec![plist::Value::String("0.35.1".to_owned())])
            .to_file_xml(dir.path().join(BUNDLE_INFO))
            .unwrap();
        assert_eq!(bundle.read(), None, "not a dictionary");
    }

    #[test]
    fn the_staged_file_sits_above_the_executables_directory() {
        let root = Path::new("/opt/ngterm");
        let expected = Some(InstalledSource::Staged(root.join(STAGED_FILE)));
        assert_eq!(InstalledSource::of_executable(&root.join("bin/ngterm")), expected);
        assert_eq!(
            InstalledSource::of_executable(&root.join("bin/ngterm (deleted)")),
            expected,
            "a binary replaced under the running process"
        );
        assert_eq!(InstalledSource::of_executable(Path::new("ngterm")), None);
    }

    #[test]
    fn a_bundle_executable_reads_the_bundle_plist_on_macos_only() {
        let bundle = Path::new("/Applications/ngterm.app");
        let source = InstalledSource::of_executable(&bundle.join("Contents/MacOS/ngterm"));
        if cfg!(target_os = "macos") {
            assert_eq!(source, Some(InstalledSource::Bundle(bundle.join(BUNDLE_INFO))));
        } else {
            assert_eq!(
                source,
                Some(InstalledSource::Staged(bundle.join("Contents").join(STAGED_FILE)))
            );
        }
        // not a bundle: the staged file, on every OS
        for executable in ["/opt/ngterm.d/Contents/MacOS/ngterm", "/opt/x.app/Other/MacOS/ngterm"] {
            let executable = Path::new(executable);
            let root = executable.parent().unwrap().parent().unwrap();
            assert_eq!(
                InstalledSource::of_executable(executable),
                Some(InstalledSource::Staged(root.join(STAGED_FILE)))
            );
        }
    }
}
