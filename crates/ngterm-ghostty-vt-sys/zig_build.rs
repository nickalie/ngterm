//! The `zig build` of libghostty-vt for one cargo target, as functions of the build script's inputs: `build.rs` runs
//! what they return, and `tests/zig_build.rs` checks them without running zig.
//!
//! herdr: `crates/ghostty-vt/build.rs` at `HERDR_REV` (`build/pins.env`), licensed under the Apache License 2.0,
//! modified. Kept: the target map, the arguments (`--prefix` under `OUT_DIR`, `-Demit-lib-vt`, the optimize mode,
//! SIMD, `-Dtarget`, `-Dversion-string`), the Apple link directory and the MSVC archive name. Changed: herdr builds a
//! source tree vendored in its repository with the zig on `PATH`; here the tree, its Zig packages and the pinned zig
//! come from the prefix `scripts/build-ghostty-vt.sh` prepared (its stamp names the zig and the Zig global cache), the
//! local cache goes under `OUT_DIR` too, the map adds Linux's glibc floor, the macOS and iOS deployment targets (the
//! iOS simulator a target of its own) and Android's API level with the NDK cargo-ndk links with, the CPU is `baseline`
//! (never the build host's), and the optimize mode and SIMD are fixed.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The prefix's stamp, relative to the prefix: its inputs as `name=value` lines (`scripts/build-ghostty-vt.sh`).
pub const STAMP: &str = "share/ngterm/ghostty-vt-STAMP";

/// The patched source tree, relative to the prefix: the build's root, with its Zig packages in `zig-pkg/`.
pub const SOURCE: &str = "src";

/// The cargo targets libghostty-vt builds for, each with its Zig triple without the OS version (herdr's map, with
/// Android, and with iOS on device and simulator, whose ABI follows the OS version: [`zig_target`]).
/// `scripts/build-ghostty-vt.sh` fetches the Zig packages of each (its `ZIG_TARGETS`, kept equal by a test).
pub const ZIG_TARGETS: [(&str, &str); 12] = [
    ("x86_64-unknown-linux-gnu", "x86_64-linux-gnu"),
    ("aarch64-unknown-linux-gnu", "aarch64-linux-gnu"),
    ("x86_64-unknown-linux-musl", "x86_64-linux-musl"),
    ("aarch64-unknown-linux-musl", "aarch64-linux-musl"),
    ("x86_64-apple-darwin", "x86_64-macos"),
    ("aarch64-apple-darwin", "aarch64-macos"),
    ("x86_64-pc-windows-msvc", "x86_64-windows-msvc"),
    ("aarch64-pc-windows-msvc", "aarch64-windows-msvc"),
    ("aarch64-linux-android", "aarch64-linux-android"),
    ("x86_64-linux-android", "x86_64-linux-android"),
    ("aarch64-apple-ios", "aarch64-ios"),
    ("aarch64-apple-ios-sim", "aarch64-ios-simulator"),
];

/// Forward an explicit libc description to the build runner as well as the compiler. Translate-C dependencies
/// read `b.libc_file`; the compiler's `ZIG_LIBC` environment alone does not set that build option.
pub fn libc_args(path: Option<&OsStr>) -> Vec<OsString> {
    path.filter(|path| !path.is_empty())
        .map(|path| vec!["--libc".into(), path.to_owned()])
        .unwrap_or_default()
}

/// The glibc a Linux gnu build targets, the libghostty build's floor: one archive serves every supported distribution.
pub const GLIBC_FLOOR: &str = "2.39";

/// The repository files whose change reruns the build script, relative to the repository root: a pin or patch change
/// moves the prefix (its key hashes them), so the rerun finds the new prefix, or fails naming the script when it is
/// not prepared yet, and builds the new tree.
pub const RERUN_FILES: [&str; 7] = [
    "build/pins.env",
    "build/patches/ghostty-vt",
    "scripts/build-ghostty-vt.sh",
    "scripts/lib/native.sh",
    "crates/ngterm-ghostty-vt-sys/build.rs",
    "crates/ngterm-ghostty-vt-sys/zig_build.rs",
    "crates/ngterm-ghostty-vt-sys/script.rs",
];

/// The environment whose change reruns the build script: the prefix override (`NGTERM_GHOSTTY_VT_PREFIX`,
/// `script.rs`), the vendor root (`NGTERM_VENDOR`, else under `HOME`), the macOS and iOS deployment targets, Android's
/// API level (`ANDROID_PLATFORM`) and what picks the NDK: `ANDROID_NDK_HOME`, cargo-ndk's sysroot ([`android_ndk`]) and
/// the SDK roots Ghostty's build searches when neither is set.
pub const RERUN_ENV: [&str; 11] = [
    "ZIG_LIBC",
    "NGTERM_GHOSTTY_VT_PREFIX",
    "NGTERM_VENDOR",
    "HOME",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "ANDROID_PLATFORM",
    "ANDROID_NDK_HOME",
    "CARGO_NDK_SYSROOT_PATH",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
];

/// What the prefix's stamp records for the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// The pinned zig the prefix was prepared with, the binary itself.
    pub zig: PathBuf,
    /// That zig's version (`LIBGHOSTTY_VT_ZIG_VERSION`), checked again before the build.
    pub zig_version: String,
    /// Zig's global cache in the vendor root, keyed by the Zig version.
    pub global_cache: PathBuf,
    /// The version string the build passes, so Ghostty's build never asks git.
    pub version_string: String,
}

impl Stamp {
    /// Parse the stamp's `name=value` lines (`#` comments and other names, such as the patches', are skipped).
    ///
    /// # Errors
    ///
    /// A missing or empty `zig`, `zig-version`, `zig-global-cache` or `version-string`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value = |name: &str| {
            text.lines()
                .filter(|line| !line.starts_with('#'))
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("the libghostty-vt prefix's stamp has no {name}"))
        };
        Ok(Self {
            zig: PathBuf::from(value("zig")?),
            zig_version: value("zig-version")?.to_owned(),
            global_cache: PathBuf::from(value("zig-global-cache")?),
            version_string: value("version-string")?.to_owned(),
        })
    }
}

/// The library's pin as two builds compare it to tell whether their libghostty-vt is the same code (the session
/// server's update handoff passes a terminal as the binary snapshot only between equal pins, `snapshot.h` promising
/// no compatibility across revisions): `LIBGHOSTTY_VT_REV` from `pins_env` (`build/pins.env`), then the FNV-1a 64 of
/// the stamp's `patch=` lines (each patch's name and sha256, in order), as 16 lower-case hex digits. The library's own
/// version string names no revision (`version-string=1.3.2-dev`), so it cannot tell two pins apart.
///
/// # Errors
///
/// `pins_env` has no `LIBGHOSTTY_VT_REV`.
pub fn pin(pins_env: &str, stamp: &str) -> Result<String, String> {
    let rev = pins_env
        .lines()
        .find_map(|line| line.trim().strip_prefix("LIBGHOSTTY_VT_REV="))
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| !value.is_empty())
        .ok_or("build/pins.env has no LIBGHOSTTY_VT_REV")?;
    let patches =
        stamp
            .lines()
            .filter(|line| line.starts_with("patch="))
            .fold(14_695_981_039_346_656_037_u64, |hash, line| {
                line.bytes().chain(std::iter::once(b'\n')).fold(hash, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
                })
            });
    Ok(format!("{rev} {patches:016x}"))
}

/// The stamp of a prepared prefix: the tree, the umbrella header and the stamp are there.
///
/// # Errors
///
/// A prefix that is missing or not prepared, with the error that names the script, or an unreadable stamp.
pub fn prepared(prefix: &Path) -> Result<Stamp, String> {
    let stamp = prefix.join(STAMP);
    let complete = prefix.join(SOURCE).join("build.zig").is_file()
        && prefix.join("include/ghostty/vt.h").is_file()
        && stamp.is_file();
    if !complete {
        return Err(format!(
            "no libghostty-vt at {}: run scripts/build-ghostty-vt.sh (it prepares the pinned libghostty-vt source there)",
            prefix.display()
        ));
    }
    let text = std::fs::read_to_string(&stamp).map_err(|err| format!("cannot read {}: {err}", stamp.display()))?;
    Stamp::parse(&text)
}

/// The oldest macOS libghostty-vt builds for: Ghostty's own floor (`src/build/Config.zig` `osVersionMin`, the oldest
/// macOS Apple still supports), which its build substitutes for any `-Dtarget` version on a macOS host. The workspace
/// links for it too (`MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml`), so no member of the archive is built for a
/// newer macOS than the binaries that link it.
pub const MACOS_FLOOR: &str = "13.0";

/// The oldest iOS libghostty-vt builds for, device and simulator alike: Ghostty's lib-vt floor (`src/build/Config.zig`
/// `osVersionMinLibVt`). The workspace links for a newer one, the app's minimum (`IPHONEOS_DEPLOYMENT_TARGET` in
/// `.cargo/config.toml`), and the archive is built for that same version, so no member of it is built for a newer iOS
/// than the binaries that link it.
pub const IOS_FLOOR: &str = "13.0";

/// The oldest Android API level libghostty-vt builds for (Android 7.0): Zig's standard library calls `preadv64`
/// whatever the level, which bionic has from API 24 on, so an archive for an older level would not link. The archive is
/// built for the level cargo-ndk links for (`ANDROID_PLATFORM`, its `--platform`, whose default 21 is refused), never a
/// newer one, so it calls nothing the NDK's libc stubs of that level lack.
pub const ANDROID_API_FLOOR: u32 = 24;

/// The NDK an Android build compiles against: `ANDROID_NDK_HOME`, else the NDK of the sysroot cargo-ndk links with
/// (`CARGO_NDK_SYSROOT_PATH`, `<ndk>/toolchains/llvm/prebuilt/<host>/sysroot`), else `None`, which leaves the choice to
/// Ghostty's build (the newest NDK under `ANDROID_HOME`, `ANDROID_SDK_ROOT` or the default SDK location).
pub fn android_ndk(ndk_home: Option<&str>, cargo_ndk_sysroot: Option<&str>) -> Option<PathBuf> {
    let set = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    set(ndk_home)
        .or_else(|| set(cargo_ndk_sysroot).and_then(|sysroot| sysroot.ancestors().nth(5).map(Path::to_path_buf)))
}

/// A `major[.minor[.patch]]` version as numbers padded to three with zeros (as rustc reads a deployment target, so
/// `13` is `13.0`), for comparing; `None` when it is not one.
fn version(text: &str) -> Option<Vec<u32>> {
    let mut parts: Vec<u32> = text.split('.').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    parts.resize(parts.len().max(3), 0);
    Some(parts)
}

/// An Apple deployment target: `requested` (from `variable`) when set and not blank, else `floor`.
///
/// # Errors
///
/// One that is not a version or is older than `floor`.
fn apple_deployment<'a>(
    variable: &str,
    system: &str,
    requested: Option<&'a str>,
    floor: &'a str,
) -> Result<&'a str, String> {
    let requested = requested
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .unwrap_or(floor);
    let supported = version(requested)
        .zip(version(floor))
        .map(|(requested, floor)| requested >= floor);
    if supported == Some(true) {
        Ok(requested)
    } else {
        Err(format!(
            "{variable}={requested}: libghostty-vt builds for {system} {floor} or newer"
        ))
    }
}

/// The `-Dtarget` of a cargo target: Linux gnu at the glibc floor, macOS at `MACOSX_DEPLOYMENT_TARGET` or else at
/// [`MACOS_FLOOR`], iOS at `IPHONEOS_DEPLOYMENT_TARGET` or else at [`IOS_FLOOR`] (on the OS part, before the
/// simulator's ABI: `aarch64-ios.26.0-simulator`), Android at `ANDROID_PLATFORM` (cargo-ndk's API level) or else at
/// [`ANDROID_API_FLOOR`], the others as mapped.
///
/// # Errors
///
/// A target outside the map, a macOS deployment target that is not a version or is older than [`MACOS_FLOOR`]
/// (Ghostty's build would build for the floor anyway on a macOS host, newer than what the binaries claim), an iOS one
/// that is not a version or is older than [`IOS_FLOOR`], or an Android API level that is not a number or is older than
/// [`ANDROID_API_FLOOR`].
pub fn zig_target(
    target: &str,
    macos_deployment_target: Option<&str>,
    ios_deployment_target: Option<&str>,
    android_platform: Option<&str>,
) -> Result<String, String> {
    let (_, triple) = ZIG_TARGETS
        .iter()
        .find(|(cargo, _)| *cargo == target)
        .ok_or_else(|| format!("libghostty-vt has no Zig target for {target} (ZIG_TARGETS in zig_build.rs)"))?;
    if triple.ends_with("-linux-gnu") {
        return Ok(format!("{triple}.{GLIBC_FLOOR}"));
    }
    if triple.ends_with("-macos") {
        let requested = apple_deployment(
            "MACOSX_DEPLOYMENT_TARGET",
            "macOS",
            macos_deployment_target,
            MACOS_FLOOR,
        )?;
        return Ok(format!("{triple}.{requested}"));
    }
    if triple.contains("-ios") {
        let requested = apple_deployment("IPHONEOS_DEPLOYMENT_TARGET", "iOS", ios_deployment_target, IOS_FLOOR)?;
        return Ok(match triple.strip_suffix("-simulator") {
            Some(device) => format!("{device}.{requested}-simulator"),
            None => format!("{triple}.{requested}"),
        });
    }
    if triple.ends_with("-android") {
        let requested = android_platform.map(str::trim).filter(|level| !level.is_empty());
        let level = match requested {
            None => ANDROID_API_FLOOR,
            Some(text) => text
                .parse::<u32>()
                .ok()
                .filter(|level| *level >= ANDROID_API_FLOOR)
                .ok_or_else(|| {
                    format!(
                        "ANDROID_PLATFORM={text}: libghostty-vt builds for Android API {ANDROID_API_FLOOR} or newer"
                    )
                })?,
        };
        return Ok(format!("{triple}.{level}"));
    }
    Ok((*triple).to_owned())
}

/// The Mach-O platform (`LC_BUILD_VERSION`'s) and the iOS version an iOS build's archive must carry: `IOS` (2) on
/// device, `IOSSIMULATOR` (7) on the simulator, and the version of its `-Dtarget` ([`zig_target`]); `None` for any
/// other target.
pub fn ios_build_version(target: &str, zig_target: &str) -> Option<(u32, String)> {
    let platform = match target {
        "aarch64-apple-ios" => 2,
        "aarch64-apple-ios-sim" => 7,
        _ => return None,
    };
    let (_, version) = zig_target.split_once("-ios.")?;
    let version = version.strip_suffix("-simulator").unwrap_or(version);
    Some((platform, version.to_owned()))
}

/// A Mach-O platform as `otool -l` prints it, by number or by name.
fn platform_number(text: &str) -> Option<u32> {
    match text {
        "IOS" => Some(2),
        "IOSSIMULATOR" => Some(7),
        _ => text.parse().ok(),
    }
}

/// Check an iOS archive from its `otool -l` text: every member carries an `LC_BUILD_VERSION` of `platform`, built for
/// `deployment` or an older iOS (Apple's linker would warn "built for newer iOS version", and the member could call
/// what the older system lacks). The host-side counterpart of the link test's macOS check, which cannot run `otool`
/// inside the simulator.
///
/// # Errors
///
/// An archive without members, or each member without a build version, of another platform or for a newer iOS.
pub fn check_ios_archive(otool: &str, platform: u32, deployment: &str) -> Result<(), String> {
    // (member, its platform, its minimum iOS), a member with no build version holding `None`s
    let mut members: Vec<(&str, Option<&str>, Option<&str>)> = Vec::new();
    for line in otool.lines() {
        if let Some(name) = line
            .strip_suffix("):")
            .and_then(|line| line.rsplit_once('('))
            .map(|(_, name)| name)
        {
            members.push((name, None, None));
        } else if let Some(member) = members.last_mut() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("platform ") {
                member.1 = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("minos ") {
                member.2 = Some(value.trim());
            }
        }
    }
    if members.is_empty() {
        return Err("otool -l lists no member in the iOS archive".to_owned());
    }
    let problems: Vec<String> = members
        .iter()
        .filter_map(|(name, found, minos)| match (found, minos) {
            (Some(found), Some(_)) if platform_number(found) != Some(platform) => {
                Some(format!("{name} is built for platform {found}, not {platform}"))
            }
            (Some(_), Some(minos)) if version(minos).zip(version(deployment)).is_none_or(|(m, d)| m > d) => Some(
                format!("{name} is built for iOS {minos}, newer than the deployment target {deployment}"),
            ),
            (Some(_), Some(_)) => None,
            _ => Some(format!("{name} has no LC_BUILD_VERSION")),
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the iOS archive does not match its target: {}",
            problems.join("; ")
        ))
    }
}

/// The inputs of one build.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The prepared prefix.
    pub prefix: &'a Path,
    /// What its stamp records.
    pub stamp: &'a Stamp,
    /// Cargo's `OUT_DIR` of this build (one per target and profile).
    pub out_dir: &'a Path,
    /// Cargo's `TARGET`.
    pub target: &'a str,
    /// `MACOSX_DEPLOYMENT_TARGET`, when set.
    pub macos_deployment_target: Option<&'a str>,
    /// `IPHONEOS_DEPLOYMENT_TARGET`, when set.
    pub ios_deployment_target: Option<&'a str>,
    /// `ANDROID_PLATFORM` (the API level cargo-ndk links for), when set.
    pub android_platform: Option<&'a str>,
    /// The NDK an Android build compiles against ([`android_ndk`]), when known.
    pub android_ndk: Option<&'a Path>,
    /// Cargo's `NUM_JOBS`, when set.
    pub jobs: Option<&'a str>,
}

/// What one build runs and links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// The zig to run.
    pub zig: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The directory it runs in: the prefix's tree.
    pub source: PathBuf,
    /// The static archive the build installs.
    pub archive: PathBuf,
    /// The directory to link from: the install's `lib/`, or on Apple one holding a copy of the archive alone (Apple's
    /// linker prefers the sibling dylib for `-l`).
    pub link_dir: PathBuf,
    /// The static library's link name: `ghostty-vt-static` on MSVC (beside the import library of the DLL),
    /// `ghostty-vt` elsewhere.
    pub link_name: &'static str,
    /// The `-Dtarget` it builds for.
    pub zig_target: String,
    /// The environment it sets: `ANDROID_NDK_HOME` for an Android build whose NDK is known (Ghostty's build reads it
    /// for bionic's headers and libc).
    pub env: Vec<(OsString, OsString)>,
}

/// The build of `inputs`: `zig build -Demit-lib-vt` over the prefix's tree, installing into and caching under
/// `OUT_DIR` (so builds for different targets or profiles never share an install), with the global cache the stamp
/// names. `ReleaseFast`: Debug's terminal integrity checks make sustained PTY output unusably slow.
///
/// # Errors
///
/// A target outside the map, or a deployment target or API level [`zig_target`] refuses.
pub fn plan(inputs: &Inputs<'_>) -> Result<Build, String> {
    let target = zig_target(
        inputs.target,
        inputs.macos_deployment_target,
        inputs.ios_deployment_target,
        inputs.android_platform,
    )?;
    let install = inputs.out_dir.join("zig-out");
    let mut args: Vec<OsString> = vec!["build".into(), "--prefix".into(), install.clone().into()];
    args.extend(["--cache-dir".into(), inputs.out_dir.join("zig-cache").into()]);
    args.extend(["--global-cache-dir".into(), inputs.stamp.global_cache.clone().into()]);
    args.extend(
        [
            "-Demit-lib-vt=true".to_owned(),
            // Rust links the archive directly; an XCFramework would require full Xcode.
            "-Demit-xcframework=false".to_owned(),
            "-Doptimize=ReleaseFast".to_owned(),
            "-Dsimd=true".to_owned(),
            "-Dcpu=baseline".to_owned(),
            format!("-Dtarget={target}"),
            format!("-Dversion-string={}", inputs.stamp.version_string),
        ]
        .map(OsString::from),
    );
    if let Some(jobs) = inputs.jobs.filter(|jobs| !jobs.is_empty()) {
        args.push(format!("-j{jobs}").into());
    }
    let msvc = inputs.target.ends_with("-windows-msvc");
    let archive = install.join("lib").join(if msvc {
        "ghostty-vt-static.lib"
    } else {
        "libghostty-vt.a"
    });
    let link_dir = if inputs.target.contains("-apple-") {
        inputs.out_dir.join("lib")
    } else {
        install.join("lib")
    };
    Ok(Build {
        zig: inputs.stamp.zig.clone(),
        args,
        source: inputs.prefix.join(SOURCE),
        archive,
        link_dir,
        link_name: if msvc { "ghostty-vt-static" } else { "ghostty-vt" },
        env: inputs
            .android_ndk
            .filter(|_| inputs.target.ends_with("-android"))
            .map(|ndk| ("ANDROID_NDK_HOME".into(), ndk.as_os_str().to_owned()))
            .into_iter()
            .collect(),
        zig_target: target,
    })
}
