//! The Linux harness (crate docs).

#[path = "cases.rs"]
mod cases;
#[path = "../support/gpus.rs"]
mod gpus;
#[path = "../support/lavapipe.rs"]
mod lavapipe;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use gpus::{Gpu, mesa_egl_vendor, mesa_gpu, pci_device};

const GPU_ENV: &str = "NGTERM_HTML_TEST_GPU";
const REQUIRE_ENV: &str = "NGTERM_REQUIRE_DISPLAY_TESTS";
/// How long one GPU's child may run.
const CHILD_TIMEOUT: Duration = Duration::from_secs(300);

pub fn main() -> ExitCode {
    if let Err(error) = pin_test_worker() {
        eprintln!("{error}; build ngterm-web before the HTML harness");
        return ExitCode::FAILURE;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--list") {
        for (name, _) in cases::CASES {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    let filter = args.iter().find(|arg| !arg.starts_with('-')).cloned();
    match std::env::var(GPU_ENV) {
        Ok(gpu) => {
            let status = cases::run(&gpu, filter.as_deref());
            // GPUI's wgpu GLES instance can crash in its TLS destructor after the display is gone, as in
            // main.rs's exit_now: flush output and coverage, then skip exit handlers.
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            ngterm_app::coverage::write_profile();
            // SAFETY: `_exit` ends this harness child without touching its torn-down graphics state.
            unsafe { libc::_exit(status) }
        }
        Err(_) => parent(&args),
    }
}

fn pin_test_worker() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("no target directory for the test worker")?;
    ngterm_wpe_client::worker::pin_worker_path(&directory.join("ngterm-web"))
}

fn parent(args: &[String]) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("the test binary cannot find itself");
        return ExitCode::FAILURE;
    };
    let mut gpus = Vec::new();
    let mut failed = Vec::new();
    match lavapipe::lavapipe_icd() {
        Some(icd) => gpus.push(Gpu {
            name: "lavapipe",
            icd,
            node: None,
        }),
        None if std::env::var_os(REQUIRE_ENV).is_some() => {
            println!("gpu lavapipe: FAILED (no lavapipe ICD, and {REQUIRE_ENV} is set)");
            failed.push("lavapipe");
        }
        None => println!("gpu lavapipe: skipped (no lavapipe ICD: scripts/fetch-lavapipe.sh)"),
    }
    match mesa_gpu() {
        Some(gpu) => gpus.push(gpu),
        None => println!("gpu mesa: skipped (no render node a Mesa Vulkan driver runs)"),
    }
    for gpu in &gpus {
        println!("gpu {} ({})", gpu.name, gpu.icd.display());
        if let Err(message) = run_child(&exe, gpu, args) {
            println!("gpu {}: FAILED ({message})", gpu.name);
            failed.push(gpu.name);
        }
    }
    if failed.is_empty() {
        println!("test result: ok");
        ExitCode::SUCCESS
    } else {
        println!("test result: FAILED on {}", failed.join(", "));
        ExitCode::FAILURE
    }
}

fn run_child(exe: &Path, gpu: &Gpu, args: &[String]) -> Result<(), String> {
    let home = tempfile::tempdir().map_err(|err| err.to_string())?;
    let mut command = std::process::Command::new(exe);
    command.args(args).env_clear().stdin(Stdio::null());
    // LLVM_PROFILE_FILE: a coverage run's profile destination (cargo llvm-cov), where the child's coverage goes
    for kept in [
        "PATH",
        "RUST_BACKTRACE",
        "RUST_LOG",
        "TMPDIR",
        REQUIRE_ENV,
        "LLVM_PROFILE_FILE",
    ] {
        if let Some(value) = std::env::var_os(kept) {
            command.env(kept, value);
        }
    }
    for (name, dir) in [
        ("HOME", ""),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_RUNTIME_DIR", "runtime"),
    ] {
        let path: PathBuf = home.path().join(dir);
        std::fs::create_dir_all(&path).map_err(|err| err.to_string())?;
        command.env(name, path);
    }
    command
        .env(GPU_ENV, gpu.name)
        .env("VK_DRIVER_FILES", &gpu.icd)
        .env("VK_ICD_FILENAMES", &gpu.icd);
    if let Some(vendor) = mesa_egl_vendor() {
        command.env("__EGL_VENDOR_LIBRARY_FILENAMES", vendor);
    }
    match &gpu.node {
        Some(node) => {
            if let Some(device) = pci_device(node) {
                command.env("ZED_DEVICE_ID", device);
            }
        }
        None => {
            command.env("LIBGL_ALWAYS_SOFTWARE", "1");
        }
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("the child did not start: {err}"))?;
    let deadline = Instant::now() + CHILD_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(status.to_string()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {CHILD_TIMEOUT:?}"));
            }
            Err(err) => return Err(format!("waiting for the child: {err}")),
        }
    }
}
