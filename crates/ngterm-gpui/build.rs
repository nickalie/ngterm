//! Points the terminal's embedded fonts (`terminal::font`) at the vendored fonts prefix: `NGTERM_GPUI_FONTS` is its
//! `share/fonts`, which `include_bytes!` reads; on Windows, links ngterm.exe's icon resource (`windows_resources`).
//!
//! The prefix path comes from `scripts/fetch-fonts.sh --print-prefix`, the one place that knows how it is keyed; this
//! script never fetches. A missing or incomplete prefix fails the build with a pointer to the script. The script runs
//! as libghostty-vt's prepare script does (`ngterm-ghostty-vt-sys`'s `script.rs`, shared): with Git Bash on Windows,
//! its MSYS path converted to the native form.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "../ngterm-ghostty-vt-sys/script.rs"]
#[expect(
    dead_code,
    reason = "the libghostty-vt prefix lookup there is that crate's; this one runs its own script"
)]
mod script;

/// The faces `terminal::font` embeds (scripts/lib/native.sh `FONT_FILES`).
const FONT_FILES: [&str; 5] = [
    "JetBrainsMono-Regular.ttf",
    "JetBrainsMono-Bold.ttf",
    "JetBrainsMono-Italic.ttf",
    "JetBrainsMono-BoldItalic.ttf",
    "SymbolsNerdFontMono-Regular.ttf",
];

fn main() -> ExitCode {
    if let Err(error) = macos_services().and_then(|()| windows_resources()) {
        println!("cargo::error={error}");
        return ExitCode::FAILURE;
    }
    match fonts() {
        Ok(fonts) => {
            for file in FONT_FILES {
                println!("cargo::rerun-if-changed={}", fonts.join(file).display());
            }
            println!("cargo::rustc-env=NGTERM_GPUI_FONTS={}", fonts.display());
        }
        Err(message) => println!("cargo::error={message}"),
    }
    ExitCode::SUCCESS
}

fn fonts() -> Result<PathBuf, String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?;
    let root = Path::new(&manifest).join("../..");
    let script = root.join("scripts/fetch-fonts.sh");
    for input in ["build/pins.env", "scripts/fetch-fonts.sh", "scripts/lib/native.sh"] {
        println!("cargo::rerun-if-changed={}", root.join(input).display());
    }
    println!("cargo::rerun-if-env-changed=NGTERM_VENDOR");
    println!("cargo::rerun-if-env-changed=HOME");

    let prefix = script::native(&script::run(&script, &["--print-prefix"], &[])?)?;
    let fonts = prefix.join("share/fonts");
    if FONT_FILES.iter().all(|file| fonts.join(file).is_file()) {
        Ok(fonts)
    } else {
        Err(format!(
            "no terminal fonts at {}: run scripts/fetch-fonts.sh (it fetches the fonts Ghostty embeds there)",
            fonts.display()
        ))
    }
}

/// On Windows, `windows/ngterm.rc` (the app icon, `resources/icons/ngterm.ico`) compiled and linked into the `ngterm`
/// binary: GPUI gives every window the executable's icon resource 1, and without one the title bar, the taskbar and
/// Alt+Tab show Windows' blank application icon.
fn windows_resources() -> Result<(), String> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?;
    let manifest = Path::new(&manifest);
    let icons = manifest.join("../../resources/icons");
    let rc = manifest.join("windows/ngterm.rc");
    println!("cargo::rerun-if-changed={}", rc.display());
    println!("cargo::rerun-if-changed={}", icons.join("ngterm.ico").display());
    embed_resource::compile_for(&rc, ["ngterm"], embed_resource::ParamsIncludeDirs([&icons]))
        .manifest_required()
        .map_err(|error| format!("{error} (the Windows SDK's rc.exe compiles ngterm.exe's icon)"))
}

/// Compile the small `AppKit` adapter with the target SDK and architecture.
fn macos_services() -> Result<(), String> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return Ok(());
    }
    println!("cargo::rerun-if-changed=src/macos.m");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    let target = std::env::var("TARGET").map_err(|error| error.to_string())?;
    let arch = if target.starts_with("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let object = out.join("macos.o");
    let status = std::process::Command::new("xcrun")
        .args([
            "clang",
            "-fobjc-arc",
            "-fblocks",
            "-arch",
            arch,
            "-mmacosx-version-min=13.0",
            "-c",
            "src/macos.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("compiling macOS services failed".into());
    }
    let status = std::process::Command::new("ar")
        .arg("crs")
        .arg(out.join("libngterm_macos.a"))
        .arg(object)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("archiving macOS services failed".into());
    }
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=ngterm_macos");
    for framework in ["Cocoa", "Carbon", "UserNotifications", "Network", "WebKit", "Metal"] {
        println!("cargo::rustc-link-lib=framework={framework}");
    }
    Ok(())
}
