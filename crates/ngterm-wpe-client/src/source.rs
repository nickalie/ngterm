//! Worker-only library/module paths and the runtime in-use lock held across the worker launch.

use std::path::PathBuf;
use std::process::Command;

/// The engine installation selected by the app before starting a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineSource {
    /// The system engine (or the development build's vendored installation).
    System,
    /// A complete, verified runtime installation.
    Runtime(PathBuf),
}

/// Worker-only startup flag: a relocatable runtime needs no development overlay.
#[doc(hidden)]
pub const RUNTIME_ENV: &str = "NGTERM_WPE_RUNTIME";

/// Runtime lock inherited by the worker alone; it restores close-on-exec before starting helpers.
#[doc(hidden)]
pub const RUNTIME_LOCK_ENV: &str = "NGTERM_WPE_RUNTIME_LOCK_FD";

impl EngineSource {
    pub(crate) fn configure(&self, command: &mut Command) {
        let Self::Runtime(root) = self else {
            // Match the system probe: only the installed library search paths may select the engine.
            command.env_remove("LD_LIBRARY_PATH");
            command.env_remove(RUNTIME_LOCK_ENV);
            return;
        };
        for (name, relative) in [
            ("LD_LIBRARY_PATH", "lib"),
            ("GIO_MODULE_DIR", "lib/gio/modules"),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", "lib/gstreamer-1.0"),
            ("GST_PLUGIN_SCANNER", "libexec/gstreamer-1.0/gst-plugin-scanner"),
            ("GST_REGISTRY_1_0", "gstreamer-registry.bin"),
            ("WEBKIT_INJECTED_BUNDLE_PATH", "lib/wpe-webkit-2.0/injected-bundle"),
        ] {
            command.env(name, root.join(relative));
        }
        for name in ["GIO_EXTRA_MODULES", "GST_PLUGIN_PATH", "GST_PLUGIN_PATH_1_0"] {
            command.env_remove(name);
        }
        command.env(RUNTIME_ENV, "1");
    }
}

impl EngineSource {
    /// Take the runtime lock before exec, then let the worker retain the same open file description.
    /// A nonblocking take fails safely when cleanup has already claimed the directory.
    pub(crate) fn lock_runtime(&self, command: &mut Command) -> std::io::Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::process::CommandExt as _;
        let Self::Runtime(root) = self else { return Ok(()) };
        let lock = std::fs::File::open(root.join(".in-use"))?;
        lock.try_lock_shared().map_err(std::io::Error::from)?;
        validate_runtime_lock(root, &lock)?;
        command.env(RUNTIME_LOCK_ENV, lock.as_raw_fd().to_string());
        let inherit_lock = move || {
            // SAFETY: fcntl only changes the child's flags on this open descriptor.
            if unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_SETFD, 0) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        };
        // SAFETY: only async-signal-safe fcntl runs after fork. Capturing the File keeps it open through spawn;
        // clearing CLOEXEC only in the child transfers its shared flock to the running worker.
        unsafe { command.pre_exec(inherit_lock) };
        Ok(())
    }
}

/// Cleanup may have unlinked this handle before the shared lock was taken. Check the inode too: the same key
/// might already have been reinstalled at the same pathname while this handle still names the removed tree.
fn validate_runtime_lock(root: &std::path::Path, lock: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let held = lock.metadata()?;
    let installed = std::fs::symlink_metadata(root.join(".in-use"))?;
    if held.dev() != installed.dev() || held.ino() != installed.ino() {
        return Err(std::io::Error::other("the web engine runtime changed before launch"));
    }
    Ok(())
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use std::fs::File;
    use std::process::Stdio;

    #[test]
    fn worker_inherits_shared_lock_until_it_exits() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".in-use");
        File::create_new(&path).unwrap();
        let mut command = Command::new("/bin/cat");
        command.env_clear().stdin(Stdio::piped()).stdout(Stdio::null());
        EngineSource::Runtime(root.path().to_owned())
            .lock_runtime(&mut command)
            .unwrap();
        let mut child = command.spawn().unwrap();
        drop(command); // the parent no longer owns any handle of the shared lock
        let other = File::options().read(true).write(true).open(path).unwrap();
        let busy = other.try_lock().is_err();
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        assert!(busy, "the exec'd worker must hold the runtime's shared flock");
        assert!(other.try_lock().is_ok());
    }

    #[test]
    fn an_unlinked_lock_cannot_protect_a_reinstalled_runtime_at_the_same_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".in-use");
        let stale = File::create_new(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let current = File::create_new(&path).unwrap();
        assert!(validate_runtime_lock(root.path(), &stale).is_err());
        assert!(validate_runtime_lock(root.path(), &current).is_ok());
    }

    #[test]
    fn runtime_launch_fails_while_cleanup_holds_the_exclusive_lock() {
        let root = tempfile::tempdir().unwrap();
        let lock = File::create_new(root.path().join(".in-use")).unwrap();
        lock.lock().unwrap();
        let mut command = Command::new("/bin/true");
        assert!(
            EngineSource::Runtime(root.path().to_owned())
                .lock_runtime(&mut command)
                .is_err()
        );
        assert!(EngineSource::System.lock_runtime(&mut command).is_ok());
    }
}
