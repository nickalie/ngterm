//! Engine source resolution. Installation and download are added independently of the system probe.

use std::path::{Component, Path, PathBuf};

/// Minimum supported WPE API, from `build/pins.env`.
pub const MIN_VERSION: &str = env!("NGTERM_WEB_ENGINE_MIN_VERSION");
/// Runtime key supplied by release packaging. Developer builds have no downloadable runtime.
pub const KEY: Option<&str> = option_env!("NGTERM_WEB_ENGINE_KEY");

/// The source selected before any engine starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    System,
    Runtime(PathBuf),
    NeedsDownload,
}

/// The runtime cache under the app's resolved state directory (`ngterm_core::paths`).
#[must_use]
pub fn runtime_root(state_dir: &Path) -> PathBuf {
    state_dir.join("web-engine")
}

/// Resolve in priority order. `probe` returns the loaded `WebKit` version or no usable executable.
/// A final key directory is published only after verified unpacking; partial directories are never selected.
#[must_use]
pub fn resolve(state_dir: &Path, key: Option<&str>, probe: impl FnOnce() -> Option<String>) -> Resolution {
    if probe()
        .and_then(|version| semver::Version::parse(&version).ok())
        .is_some_and(|version| {
            semver::Version::parse(MIN_VERSION)
                .is_ok_and(|minimum| version.major == minimum.major && version >= minimum)
        })
    {
        return Resolution::System;
    }
    if let Some(key) = key.filter(|key| valid_key(key)) {
        let directory = runtime_root(state_dir).join(key);
        if complete(&directory) {
            return Resolution::Runtime(directory);
        }
    }
    Resolution::NeedsDownload
}

fn valid_key(key: &str) -> bool {
    !key.ends_with(".partial") && {
        let mut components = Path::new(key).components();
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
    }
}

fn complete(directory: &Path) -> bool {
    [
        "MANIFEST",
        "lib/libWPEWebKit-2.0.so.1",
        "lib/wpe-webkit-2.0/WPEWebProcess",
        "lib/wpe-webkit-2.0/WPENetworkProcess",
        "libexec/gstreamer-1.0/gst-plugin-scanner",
    ]
    .iter()
    .all(|path| directory.join(path).is_file())
        && [
            "lib/gio/modules",
            "lib/gstreamer-1.0",
            "lib/wpe-webkit-2.0/injected-bundle",
        ]
        .iter()
        .all(|path| directory.join(path).is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(state: &Path, key: &str) -> PathBuf {
        let root = runtime_root(state).join(key);
        for file in [
            "MANIFEST",
            "lib/libWPEWebKit-2.0.so.1",
            "lib/wpe-webkit-2.0/WPEWebProcess",
            "lib/wpe-webkit-2.0/WPENetworkProcess",
            "libexec/gstreamer-1.0/gst-plugin-scanner",
        ] {
            let file = root.join(file);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "fixture").unwrap();
        }
        for directory in [
            "lib/gio/modules",
            "lib/gstreamer-1.0",
            "lib/wpe-webkit-2.0/injected-bundle",
        ] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        root
    }

    #[test]
    fn system_precedes_cached_runtime_and_absent_or_old_system_falls_back() {
        let state = tempfile::tempdir().unwrap();
        let root = runtime(state.path(), "key");
        for version in [MIN_VERSION, "2.99.0"] {
            assert_eq!(
                resolve(state.path(), Some("key"), || Some(version.to_owned())),
                Resolution::System
            );
        }
        for version in [None, Some("2.48.3"), Some("loader error"), Some("3.0.0")] {
            assert_eq!(
                resolve(state.path(), Some("key"), || version.map(str::to_owned)),
                Resolution::Runtime(root.clone())
            );
        }
        assert_eq!(
            resolve(state.path(), Some("other-key"), || None),
            Resolution::NeedsDownload
        );
        assert_eq!(resolve(state.path(), None, || None), Resolution::NeedsDownload);
    }

    #[test]
    fn partial_and_incomplete_runtimes_are_not_selected() {
        let state = tempfile::tempdir().unwrap();
        runtime(state.path(), "key.partial");
        assert_eq!(resolve(state.path(), Some("key"), || None), Resolution::NeedsDownload);
        assert_eq!(
            resolve(state.path(), Some("key.partial"), || None),
            Resolution::NeedsDownload
        );
        let root = runtime(state.path(), "key");
        std::fs::remove_file(root.join("MANIFEST")).unwrap();
        assert_eq!(resolve(state.path(), Some("key"), || None), Resolution::NeedsDownload);
    }

    #[test]
    fn unsafe_keys_cannot_select_complete_runtimes_outside_the_cache() {
        let state = tempfile::tempdir().unwrap();
        let absolute = state.path().join("absolute-key");
        for key in ["", ".", "..", "../key", absolute.to_str().unwrap()] {
            let directory = runtime(state.path(), key);
            assert!(
                complete(&directory),
                "fixture for {key:?} must be selectable without the key guard"
            );
            assert_eq!(resolve(state.path(), Some(key), || None), Resolution::NeedsDownload);
        }
    }
}
