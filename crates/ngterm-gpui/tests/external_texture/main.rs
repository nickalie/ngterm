//! The external texture element against GPUI's real renderer, patched (`vendor/gpui-pre-wgpu`): dmabufs imported into
//! its Vulkan device (acquired from the foreign queue family and copied on the GPU), CPU pixels by damaged rectangle,
//! clipping, the paint order, frame outcomes and import lifetime, rendered by GPUI's headless wgpu renderer and read
//! back; and the coverage masks the terminal's sprites paint with (`Window::paint_alpha_mask`, patch 0003). No
//! display and no compositor: Vulkan only.
//!
//! GPUI's headless renderer opens one device per process, so this harness (no libtest) runs each GPU in a child process
//! of its own, with an environment built from scratch (no display, no session bus, no `NGTERM_*`/`AGTERM_*`/`ZMX_*`):
//! `lavapipe` (Mesa's software Vulkan) and `mesa`, the first render node a Mesa Vulkan driver runs (Intel, AMD), where
//! there is one. GPUI is kept to the Vulkan adapter: on lavapipe Mesa's GL is forced to software (a CPU adapter like
//! lavapipe, which Vulkan then outranks), on hardware `ZED_DEVICE_ID` names the node's PCI device (GL needs the
//! hardware there, for the GL producer). The synthetic dmabufs come from `/dev/udmabuf` and, on hardware, from GBM
//! buffers a GL context clears.
//!
//! A missing piece (no lavapipe, no udmabuf access, a lavapipe without dmabuf import) skips its cases with a message,
//! and fails them instead under `NGTERM_REQUIRE_DISPLAY_TESTS`, as CI sets it; only the hardware-only case on lavapipe
//! is never required.
//!
//! Arguments: a substring selects cases by name; `--list` lists them.

#[cfg(target_os = "linux")]
mod buffers;
#[cfg(target_os = "linux")]
mod cases;
#[cfg(target_os = "linux")]
mod gl;
#[cfg(target_os = "linux")]
#[path = "../support/gpus.rs"]
mod gpus;
#[cfg(target_os = "linux")]
#[path = "../support/lavapipe.rs"]
mod lavapipe;

#[cfg(target_os = "linux")]
use gpus::{Gpu, mesa_egl_vendor, mesa_gpu, pci_device};
#[cfg(target_os = "linux")]
use lavapipe::lavapipe_icd;

#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, ExitCode, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
const GPU_ENV: &str = "NGTERM_EXTERNAL_TEXTURE_GPU";
#[cfg(target_os = "linux")]
const NODE_ENV: &str = "NGTERM_EXTERNAL_TEXTURE_NODE";
#[cfg(target_os = "linux")]
const REQUIRE_ENV: &str = "NGTERM_REQUIRE_DISPLAY_TESTS";
/// How long one GPU's child may run.
#[cfg(target_os = "linux")]
const CHILD_TIMEOUT: Duration = Duration::from_secs(300);

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--list") {
        for (name, _) in cases::CASES {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    let filter = args.iter().find(|arg| !arg.starts_with('-')).cloned();
    match std::env::var(GPU_ENV) {
        Ok(gpu) => cases::run(&gpu, std::env::var_os(NODE_ENV).map(PathBuf::from), filter.as_deref()),
        Err(_) => parent(&args),
    }
}

#[cfg(target_os = "linux")]
fn parent(args: &[String]) -> ExitCode {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("the test binary cannot find itself");
        return ExitCode::FAILURE;
    };
    let mut gpus = Vec::new();
    let mut failed = Vec::new();
    match lavapipe_icd() {
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

#[cfg(target_os = "linux")]
fn run_child(exe: &Path, gpu: &Gpu, args: &[String]) -> Result<(), String> {
    let mut command = Command::new(exe);
    command.args(args).env_clear().stdin(Stdio::null());
    for kept in ["PATH", "HOME", "RUST_BACKTRACE", "RUST_LOG", "TMPDIR", REQUIRE_ENV] {
        if let Some(value) = std::env::var_os(kept) {
            command.env(kept, value);
        }
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
            command.env(NODE_ENV, node);
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

/// Dmabufs, Vulkan's foreign queue family and the external surfaces are Linux's (GPUI patch 1 is Linux only): nothing
/// to run elsewhere.
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("SKIPPED: external textures are Linux only");
}
