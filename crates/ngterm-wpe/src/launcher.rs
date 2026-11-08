//! Where a program that runs WPE `WebKit` finds its helper processes.
//!
//! A distribution build has its helper path compiled in (`/usr/lib/wpe-webkit-2.0/WPEWebProcess` on Arch; release
//! builds read no `WEBKIT_EXEC_PATH`), so the library and its helpers must sit where the package put them. That holds
//! for the system's `wpewebkit`. The vendored package of `scripts/fetch-wpe.sh` lives in a prefix of its own, and its
//! pages run only with that prefix's `lib/` overlaid on `/usr/lib`, read-only, in a mount namespace of the engine's
//! own (`bwrap --ro-overlay`, no root; Task 2's spike launcher). That namespace is a user namespace with
//! `PR_SET_NO_NEW_PRIVS`, inherited by everything started inside it, so only the engine may run there: the engine
//! runs in a worker under the overlay ([`crate::Host::start`] and [`crate::Host::start_worker`], crate `worker`), and
//! the consumer never enters it itself.
//! [`command`] builds the overlay's launch of a program for an engine-only test process.
//!
//! Release bundles use the system-provider launch with a patched, relocatable WPE library. Their marker enables
//! module paths on the engine worker's command only; the app and its terminal processes keep their environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The library directory the build found WPE `WebKit` in (`build.rs`).
const LIBDIR: &str = env!("NGTERM_WPE_LIBDIR");
/// Where a distribution build looks for its helpers, and so where a vendored prefix's `lib/` is overlaid.
const SYSTEM_LIBDIR: &str = "/usr/lib";
/// The manifest `scripts/fetch-wpe.sh` leaves in a vendored prefix, relative to its `lib/`.
const VENDORED_MANIFEST: &str = "../share/ngterm/wpe-PINS";
/// The release marker written by `scripts/package-release.sh`, relative to the binary's install prefix.
const BUNDLE_MARKER: &str = "share/ngterm/bundled-wpe";

/// Where WPE `WebKit` comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    /// A system installation or release bundle, with helpers where its library looks for them.
    System,
    /// The vendored package, its library directory overlaid on `/usr/lib` for the engine.
    Vendored { libdir: PathBuf },
}

/// The provider of the WPE `WebKit` this crate was built against.
#[must_use]
pub fn provider() -> Provider {
    provider_for(Path::new(LIBDIR))
}

fn provider_for(libdir: &Path) -> Provider {
    if libdir.join(VENDORED_MANIFEST).is_file() {
        Provider::Vendored {
            libdir: libdir.to_path_buf(),
        }
    } else {
        Provider::System
    }
}

/// Configure only the engine worker with a release bundle's modules. Keeping these
/// variables off the app preserves the environment of terminal panes and session servers.
pub(crate) fn bundle_environment(command: &mut Command, executable: &Path) {
    let Some(prefix) = executable.parent().and_then(Path::parent) else {
        return;
    };
    if !prefix.join(BUNDLE_MARKER).is_file() {
        return;
    }
    for (name, relative) in [
        ("GIO_EXTRA_MODULES", "lib/gio/modules"),
        ("GST_PLUGIN_SYSTEM_PATH_1_0", "lib/gstreamer-1.0"),
        ("GST_PLUGIN_SCANNER", "libexec/gstreamer-1.0/gst-plugin-scanner"),
        ("WEBKIT_INJECTED_BUNDLE_PATH", "lib/wpe-webkit-2.0/injected-bundle"),
    ] {
        command.env(name, prefix.join(relative));
    }
}

/// A command that runs `program` where the engine's helpers are found: as is for the system's WPE `WebKit`, under
/// `bwrap` with the vendored prefix overlaid otherwise (the working directory kept, the program ended with its parent).
/// For a program that runs nothing but the engine (a test of it): whatever it starts inherits the overlay's user
/// namespace and `PR_SET_NO_NEW_PRIVS`.
#[must_use]
pub fn command(program: &Path) -> Command {
    match provider() {
        Provider::System => Command::new(program),
        Provider::Vendored { libdir } => {
            let cwd = std::env::current_dir().ok();
            let mut command = Command::new("bwrap");
            command.args(overlay_args(&libdir, cwd.as_deref(), true, &[], program));
            command
        }
    }
}

/// `bwrap`'s arguments for running `program` with `libdir` overlaid on `/usr/lib`, bwrap's own `options` last before
/// the program; `die_with_parent` ends the program with the thread that started `bwrap`.
pub(crate) fn overlay_args(
    libdir: &Path,
    cwd: Option<&Path>,
    die_with_parent: bool,
    options: &[OsString],
    program: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["--dev-bind", "/", "/"].into_iter().map(OsString::from).collect();
    if die_with_parent {
        args.push("--die-with-parent".into());
    }
    args.push("--overlay-src".into());
    args.push(libdir.into());
    args.extend(["--overlay-src", SYSTEM_LIBDIR, "--ro-overlay", SYSTEM_LIBDIR].map(OsString::from));
    if let Some(cwd) = cwd {
        args.push("--chdir".into());
        args.push(cwd.into());
    }
    args.extend(options.iter().cloned());
    args.push("--".into());
    args.push(program.into());
    args
}

/// Re-executes a vendored worker under its helper overlay.
/// # Safety
/// Call first in main, before any other thread exists: changes the environment.
pub unsafe fn prepare_worker() {
    use std::os::unix::process::CommandExt as _;
    // The dynamic loader already consumed this; helpers and host tools must not inherit it.
    // SAFETY: caller guarantees that no other thread can read the environment.
    unsafe { std::env::remove_var("LD_LIBRARY_PATH") };
    let runtime = std::env::var_os(ngterm_wpe_client::RUNTIME_ENV).is_some();
    // SAFETY: caller guarantees a single thread; this flag must not reach WebKit's processes.
    unsafe { std::env::remove_var(ngterm_wpe_client::RUNTIME_ENV) };
    if runtime {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    if !helpers_reachable() {
        let Provider::Vendored { libdir } = provider() else {
            return;
        };
        let image = std::env::var(crate::worker::WORKER_IMAGE_ENV)
            .ok()
            .map_or_else(|| executable.clone(), |fd| PathBuf::from(format!("/proc/self/fd/{fd}")));
        let mut command = worker_overlay(&libdir, &image);
        command.args(std::env::args_os().skip(1));
        let error = command.exec();
        eprintln!("ngterm-web: cannot enter the WPE helper overlay: {error}");
        crate::exit_process(2);
    }
    let mut environment = Command::new(&executable);
    bundle_environment(&mut environment, &executable);
    for (name, value) in environment.get_envs() {
        if let Some(value) = value {
            // SAFETY: caller guarantees a single thread.
            unsafe { std::env::set_var(name, value) };
        }
    }
}

fn worker_overlay(libdir: &Path, image: &Path) -> Command {
    let mut command = Command::new("bwrap");
    command.args(overlay_args(
        libdir,
        std::env::current_dir().ok().as_deref(),
        false,
        &[],
        image,
    ));
    command
}

/// Whether the engine's web process is where this build's WPE `WebKit` looks for it: true for the system's, and for
/// the vendored package only inside its overlay ([`command`], the worker).
#[must_use]
pub fn helpers_reachable() -> bool {
    match provider() {
        Provider::System => true,
        Provider::Vendored { libdir } => {
            // the overlay shows the prefix's own file: same size, same modification time
            let stamp = |dir: &Path| {
                let metadata = std::fs::metadata(dir.join("wpe-webkit-2.0/WPEWebProcess")).ok()?;
                Some((metadata.len(), metadata.modified().ok()?))
            };
            stamp(&libdir).is_some_and(|prefix| stamp(Path::new(SYSTEM_LIBDIR)) == Some(prefix))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_reexec_uses_the_vendored_overlay_without_dying_with_its_launcher_thread() {
        let command = worker_overlay(Path::new("/vendor/lib"), Path::new("/proc/self/fd/12"));
        assert_eq!(command.get_program(), "bwrap");
        assert_eq!(command.get_args().last().unwrap(), "/proc/self/fd/12");
        assert!(command.get_args().any(|arg| arg == "/vendor/lib"));
        assert!(!command.get_args().any(|arg| arg == "--die-with-parent"));
    }

    #[test]
    fn bundled_module_paths_are_scoped_to_the_worker_command() {
        let names = [
            "GIO_EXTRA_MODULES",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_SCANNER",
            "WEBKIT_INJECTED_BUNDLE_PATH",
        ];
        let inherited = names.map(std::env::var_os);
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("usr/bin/ngterm");
        let mut command = Command::new("engine");
        bundle_environment(&mut command, &executable);
        assert_eq!(command.get_envs().count(), 0, "ordinary builds keep their environment");
        let own = root.path().join("usr/share/ngterm");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("bundled-wpe"), "").unwrap();
        bundle_environment(&mut command, &executable);
        let env: Vec<_> = command.get_envs().collect();
        assert_eq!(env.len(), 4);
        for (name, relative) in [
            ("GIO_EXTRA_MODULES", "lib/gio/modules"),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", "lib/gstreamer-1.0"),
            ("GST_PLUGIN_SCANNER", "libexec/gstreamer-1.0/gst-plugin-scanner"),
            ("WEBKIT_INJECTED_BUNDLE_PATH", "lib/wpe-webkit-2.0/injected-bundle"),
        ] {
            let (_, value) = env.iter().find(|(key, _)| *key == name).unwrap();
            assert_eq!(Path::new(value.unwrap()), root.path().join("usr").join(relative));
        }
        assert_eq!(
            names.map(std::env::var_os),
            inherited,
            "the app's environment is unchanged"
        );
    }

    #[test]
    fn a_prefix_with_fetch_wpe_s_manifest_is_vendored() {
        let prefix = tempfile::tempdir().unwrap();
        let libdir = prefix.path().join("lib");
        std::fs::create_dir_all(&libdir).unwrap();
        assert_eq!(provider_for(&libdir), Provider::System);
        std::fs::create_dir_all(prefix.path().join("share/ngterm")).unwrap();
        let manifest = prefix.path().join("share/ngterm/wpe-PINS");
        std::fs::create_dir(&manifest).unwrap();
        assert_eq!(provider_for(&libdir), Provider::System, "a directory is no manifest");
        std::fs::remove_dir(&manifest).unwrap();
        std::fs::write(prefix.path().join("share/ngterm/wpe-PINS"), "WPE_VERSION=2.54.1\n").unwrap();
        assert_eq!(provider_for(&libdir), Provider::Vendored { libdir });
        assert_eq!(provider_for(Path::new("/usr/lib")), Provider::System);
    }

    #[test]
    fn the_overlay_puts_the_prefix_over_usr_lib_and_keeps_the_working_directory() {
        let args = overlay_args(
            Path::new("/v/wpe/lib"),
            Some(Path::new("/work")),
            true,
            &[OsString::from("--json-status-fd"), OsString::from("3")],
            Path::new("/bin/probe"),
        );
        let args: Vec<&str> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "--dev-bind",
                "/",
                "/",
                "--die-with-parent",
                "--overlay-src",
                "/v/wpe/lib",
                "--overlay-src",
                "/usr/lib",
                "--ro-overlay",
                "/usr/lib",
                "--chdir",
                "/work",
                "--json-status-fd",
                "3",
                "--",
                "/bin/probe"
            ]
        );
        let worker = overlay_args(Path::new("/v/wpe/lib"), None, false, &[], Path::new("/bin/probe"));
        assert!(!worker.iter().any(|arg| arg == "--chdir"));
        assert!(
            !worker.iter().any(|arg| arg == "--die-with-parent"),
            "a worker ends with its socket, not with the thread that started it"
        );
    }
    #[test]
    fn a_command_runs_the_requested_program_and_passes_its_arguments() {
        let output = command(Path::new("/bin/printf"))
            .arg("%s")
            .arg("a single argument")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"a single argument");
    }

    #[test]
    fn a_bare_executable_has_no_bundle_prefix() {
        let mut command = Command::new("engine");
        bundle_environment(&mut command, Path::new("engine"));
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn a_private_namespace_exercises_the_vendored_launcher_and_missing_helpers() {
        const CHILD: &str = "WPE_LAUNCHER_NAMESPACE_PROBE";
        const NAME: &str = "launcher::tests::a_private_namespace_exercises_the_vendored_launcher_and_missing_helpers";
        if let Ok(mode) = std::env::var(CHILD) {
            assert_eq!(provider(), Provider::Vendored { libdir: LIBDIR.into() });
            assert_eq!(helpers_reachable(), mode == "present");
            if mode == "present" {
                // Only the marker is emulated here: /usr/lib cannot actually be overlaid on itself.
                let launch = command(Path::new("/bin/printf"));
                assert_eq!(launch.get_program(), "bwrap");
                assert_eq!(launch.get_args().last().unwrap(), "/bin/printf");
                assert!(launch.get_args().any(|arg| arg == "--die-with-parent"));
                let args: Vec<_> = launch.get_args().collect();
                let chdir = args.iter().position(|arg| *arg == "--chdir").unwrap();
                assert_eq!(Path::new(args[chdir + 1]), std::env::current_dir().unwrap());
            }
            crate::exit_process(77);
        }
        if LIBDIR != SYSTEM_LIBDIR {
            eprintln!("the private system-provider namespace fixture needs LIBDIR=/usr/lib");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let manifest = root.path().join("wpe-PINS");
        std::fs::write(&manifest, "WPE_VERSION=test\n").unwrap();
        let empty_helpers = root.path().join("empty-helpers");
        std::fs::create_dir(&empty_helpers).unwrap();
        for mode in ["present", "missing"] {
            let mut probe = Command::new("bwrap");
            probe.args(["--dev-bind", "/", "/", "--tmpfs", "/usr/share", "--ro-bind"]);
            probe.arg(&manifest).arg("/usr/share/ngterm/wpe-PINS");
            if mode == "missing" {
                probe
                    .arg("--ro-bind")
                    .arg(&empty_helpers)
                    .arg("/usr/lib/wpe-webkit-2.0");
            }
            probe
                .arg("--")
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", NAME, "--nocapture"]);
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("NGTERM_") {
                    probe.env_remove(key);
                }
            }
            let output = probe.env(CHILD, mode).output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(77),
                "{mode}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
