//! The build script's Zig build of libghostty-vt (`zig_build.rs`). Its pure functions are checked without running
//! zig: the target map, the arguments and directories of a build, the stamp, the reruns and the list of targets the
//! prepare script fetches packages for. The error a missing prefix fails the build with comes from a nested
//! `cargo build` against an empty vendor root, which fails before zig runs. The offline check needs the prefix
//! `scripts/build-ghostty-vt.sh` prepared and runs its zig to configure every mapped target with networking off.

#[path = "../script.rs"]
mod script;
#[path = "../zig_build.rs"]
mod zig_build;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use zig_build::{Build, Inputs, Stamp};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stamp() -> Stamp {
    Stamp {
        zig: PathBuf::from("/vendor/zig-x86_64-linux-0.17.0/zig"),
        zig_version: "0.17.0".to_owned(),
        global_cache: PathBuf::from("/vendor/zig-cache-0.17.0"),
        version_string: "1.4.0".to_owned(),
    }
}

/// The environment entry naming [`stamp`]'s global cache, which every build sets.
fn global_cache() -> (OsString, OsString) {
    ("ZIG_GLOBAL_CACHE_DIR".into(), "/vendor/zig-cache-0.17.0".into())
}

fn plan(out_dir: &Path, target: &str, macos: Option<&str>) -> Build {
    let stamp = stamp();
    zig_build::plan(&Inputs {
        prefix: Path::new("/vendor/ghostty-vt-0123456789abcdef"),
        stamp: &stamp,
        out_dir,
        target,
        macos_deployment_target: macos,
        ios_deployment_target: None,
        android_platform: None,
        android_ndk: None,
        jobs: Some("10"),
    })
    .unwrap()
}

/// The value following `flag` in `args`.
fn after<'a>(args: &'a [OsString], flag: &str) -> &'a Path {
    let index = args.iter().position(|arg| arg == flag).unwrap();
    Path::new(&args[index + 1])
}

#[test]
fn the_map_targets_linux_at_the_glibc_floor_and_macos_at_its_deployment_target() {
    let target = |cargo, macos| zig_build::zig_target(cargo, macos, None, None).unwrap();
    assert_eq!(target("x86_64-unknown-linux-gnu", None), "x86_64-linux-gnu.2.39");
    assert_eq!(target("aarch64-unknown-linux-gnu", None), "aarch64-linux-gnu.2.39");
    assert_eq!(target("x86_64-unknown-linux-musl", None), "x86_64-linux-musl");
    // MACOSX_DEPLOYMENT_TARGET, else Ghostty's floor; never below it
    assert_eq!(target("aarch64-apple-darwin", None), "aarch64-macos.13.0");
    assert_eq!(target("x86_64-apple-darwin", None), "x86_64-macos.13.0");
    assert_eq!(target("aarch64-apple-darwin", Some("13.3")), "aarch64-macos.13.3");
    assert_eq!(target("aarch64-apple-darwin", Some("14")), "aarch64-macos.14");
    // a major-only version is the floor itself, as rustc pads it with zeros
    assert_eq!(target("aarch64-apple-darwin", Some("13")), "aarch64-macos.13");
    assert_eq!(target("aarch64-apple-darwin", Some("13.0.0")), "aarch64-macos.13.0.0");
    assert_eq!(target("aarch64-apple-darwin", Some(" ")), "aarch64-macos.13.0");
    for refused in ["11.0", "12.7.6", "13.x"] {
        let error = zig_build::zig_target("aarch64-apple-darwin", Some(refused), None, None).unwrap_err();
        assert!(
            error.contains(&format!("MACOSX_DEPLOYMENT_TARGET={refused}")),
            "{error}"
        );
        assert!(error.contains("macOS 13.0 or newer"), "{error}");
    }
    assert_eq!(target("x86_64-pc-windows-msvc", None), "x86_64-windows-msvc");
    assert_eq!(target("aarch64-pc-windows-msvc", None), "aarch64-windows-msvc");
    let unmapped = zig_build::zig_target("riscv64gc-unknown-linux-gnu", None, None, None).unwrap_err();
    assert!(unmapped.contains("riscv64gc-unknown-linux-gnu"), "{unmapped}");
}

#[test]
fn android_builds_for_cargo_ndks_api_level_and_its_ndk() {
    let target = |cargo, level| zig_build::zig_target(cargo, None, None, level).unwrap();
    assert_eq!(target("aarch64-linux-android", None), "aarch64-linux-android.24");
    assert_eq!(target("x86_64-linux-android", Some("26")), "x86_64-linux-android.26");
    assert_eq!(target("aarch64-linux-android", Some(" ")), "aarch64-linux-android.24");
    // cargo-ndk's default --platform among them: Zig's std calls preadv64, which bionic has from API 24 on
    for refused in ["21", "23", "x"] {
        let error = zig_build::zig_target("aarch64-linux-android", None, None, Some(refused)).unwrap_err();
        assert!(error.contains(&format!("ANDROID_PLATFORM={refused}")), "{error}");
    }
    let sysroot = "/sdk/ndk/27.1.12297006/toolchains/llvm/prebuilt/linux-x86_64/sysroot";
    assert_eq!(
        zig_build::android_ndk(None, Some(sysroot)),
        Some(PathBuf::from("/sdk/ndk/27.1.12297006"))
    );
    assert_eq!(
        zig_build::android_ndk(Some("/ndk"), Some(sysroot)),
        Some(PathBuf::from("/ndk"))
    );
    assert_eq!(zig_build::android_ndk(Some(""), None), None);
    let stamp = stamp();
    let ndk = Path::new("/sdk/ndk/27.1.12297006");
    let inputs = |target| Inputs {
        prefix: Path::new("/vendor/ghostty-vt-0123456789abcdef"),
        stamp: &stamp,
        out_dir: Path::new("/out"),
        target,
        macos_deployment_target: None,
        ios_deployment_target: None,
        android_platform: Some("24"),
        android_ndk: Some(ndk),
        jobs: None,
    };
    let android = zig_build::plan(&inputs("aarch64-linux-android")).unwrap();
    assert!(
        android
            .args
            .contains(&OsString::from("-Dtarget=aarch64-linux-android.24"))
    );
    assert_eq!(
        android.env,
        [
            global_cache(),
            (OsString::from("ANDROID_NDK_HOME"), ndk.as_os_str().to_owned())
        ]
    );
    assert_eq!(android.archive, Path::new("/out/zig-out/lib/libghostty-vt.a"));
    assert_eq!(
        zig_build::plan(&inputs("x86_64-unknown-linux-gnu")).unwrap().env,
        [global_cache()]
    );
}

/// iOS builds for `IPHONEOS_DEPLOYMENT_TARGET` (`.cargo/config.toml`), else Ghostty's libghostty-vt floor, never below
/// it; the version goes on the OS part, before the simulator's ABI. Each links a copy of its archive alone, as macOS.
#[test]
fn ios_builds_for_its_deployment_target_on_device_and_simulator() {
    let target = |cargo, ios| zig_build::zig_target(cargo, None, ios, None).unwrap();
    assert_eq!(target("aarch64-apple-ios", Some("26.0")), "aarch64-ios.26.0");
    assert_eq!(
        target("aarch64-apple-ios-sim", Some("26.0")),
        "aarch64-ios.26.0-simulator"
    );
    assert_eq!(target("aarch64-apple-ios", None), "aarch64-ios.13.0");
    assert_eq!(target("aarch64-apple-ios-sim", Some(" ")), "aarch64-ios.13.0-simulator");
    assert_eq!(target("aarch64-apple-ios", Some("13")), "aarch64-ios.13");
    // the macOS deployment target is not the iOS one, nor the other way round
    assert_eq!(
        zig_build::zig_target("aarch64-apple-ios", Some("15.0"), Some("26.0"), None).unwrap(),
        "aarch64-ios.26.0"
    );
    assert_eq!(
        zig_build::zig_target("aarch64-apple-darwin", None, Some("26.0"), None).unwrap(),
        "aarch64-macos.13.0"
    );
    for cargo in ["aarch64-apple-ios", "aarch64-apple-ios-sim"] {
        for refused in ["12.4", "9", "26.x"] {
            let error = zig_build::zig_target(cargo, None, Some(refused), None).unwrap_err();
            assert!(
                error.contains(&format!("IPHONEOS_DEPLOYMENT_TARGET={refused}")),
                "{error}"
            );
            assert!(error.contains("iOS 13.0 or newer"), "{error}");
        }
    }
    let stamp = stamp();
    for (cargo, zig_target) in [
        ("aarch64-apple-ios", "aarch64-ios.26.0"),
        ("aarch64-apple-ios-sim", "aarch64-ios.26.0-simulator"),
    ] {
        let build = zig_build::plan(&Inputs {
            prefix: Path::new("/vendor/ghostty-vt-0123456789abcdef"),
            stamp: &stamp,
            out_dir: Path::new("/out"),
            target: cargo,
            macos_deployment_target: Some("13.0"),
            ios_deployment_target: Some("26.0"),
            android_platform: None,
            android_ndk: None,
            jobs: None,
        })
        .unwrap();
        assert!(build.args.contains(&OsString::from(format!("-Dtarget={zig_target}"))));
        assert_eq!(build.zig_target, zig_target);
        assert_eq!(build.archive, Path::new("/out/zig-out/lib/libghostty-vt.a"));
        assert_eq!(build.link_dir, Path::new("/out/lib"));
        assert_eq!(build.link_name, "ghostty-vt");
        assert_eq!(build.env, [global_cache()]);
    }
}

/// `otool -l` of an archive with a member per `(name, platform, minos)`, each with a segment before its build version;
/// a `None` platform leaves the member without one.
fn otool(members: &[(&str, Option<&str>, &str)]) -> String {
    let mut text = "Archive : /out/lib/libghostty-vt.a\n".to_owned();
    for (name, platform, minos) in members {
        text.push_str(&format!(
            "/out/lib/libghostty-vt.a({name}):\nLoad command 0\n      cmd LC_SEGMENT_64\n  cmdsize 1032\n"
        ));
        if let Some(platform) = platform {
            text.push_str(&format!(
                "Load command 1\n      cmd LC_BUILD_VERSION\n  cmdsize 24\n platform {platform}\n    minos {minos}\n      \
                 sdk n/a\n   ntools 0\n"
            ));
        }
    }
    text
}

/// What an iOS archive is checked against on the host (no test can run `otool` inside the simulator): the platform of
/// its target and the version of its `-Dtarget`. Every member must carry that platform and be built for that iOS or an
/// older one.
#[test]
fn an_ios_archive_is_checked_for_its_platform_and_minimum_ios() {
    assert_eq!(
        zig_build::ios_build_version("aarch64-apple-ios", "aarch64-ios.26.0"),
        Some((2, "26.0".to_owned()))
    );
    assert_eq!(
        zig_build::ios_build_version("aarch64-apple-ios-sim", "aarch64-ios.26.0-simulator"),
        Some((7, "26.0".to_owned()))
    );
    assert_eq!(
        zig_build::ios_build_version("aarch64-apple-darwin", "aarch64-macos.13.0"),
        None
    );
    assert_eq!(
        zig_build::ios_build_version("x86_64-unknown-linux-gnu", "x86_64-linux-gnu.2.39"),
        None
    );

    let check = zig_build::check_ios_archive;
    let good = otool(&[("a.o", Some("7"), "26.0"), ("b.o", Some("7"), "13.0")]);
    assert_eq!(check(&good, 7, "26.0"), Ok(()));
    // a platform printed by its name is read as its number
    assert_eq!(
        check(&otool(&[("a.o", Some("IOSSIMULATOR"), "26.0")]), 7, "26.0"),
        Ok(())
    );
    assert_eq!(check(&otool(&[("a.o", Some("IOS"), "26")]), 2, "26.0"), Ok(()));

    let device_on_sim = check(
        &otool(&[("a.o", Some("7"), "26.0"), ("b.o", Some("2"), "26.0")]),
        7,
        "26.0",
    );
    let error = device_on_sim.unwrap_err();
    assert!(error.contains("b.o") && error.contains("platform 2"), "{error}");
    let newer = check(
        &otool(&[("a.o", Some("2"), "26.0"), ("c.o", Some("2"), "26.1")]),
        2,
        "26.0",
    )
    .unwrap_err();
    assert!(
        newer.contains("c.o") && newer.contains("iOS 26.1") && newer.contains("26.0"),
        "{newer}"
    );
    let missing = check(&otool(&[("a.o", Some("2"), "26.0"), ("d.o", None, "")]), 2, "26.0").unwrap_err();
    assert!(
        missing.contains("d.o") && missing.contains("LC_BUILD_VERSION"),
        "{missing}"
    );
    let empty = check("Archive : /out/lib/libghostty-vt.a\n", 2, "26.0").unwrap_err();
    assert!(empty.contains("no member"), "{empty}");
}

#[test]
fn a_build_installs_and_caches_under_its_own_out_dir() {
    let linux_out = Path::new("/target/debug/build/ngterm-ghostty-vt-sys-1/out");
    let arm_out = Path::new("/target/aarch64-unknown-linux-gnu/debug/build/ngterm-ghostty-vt-sys-2/out");
    for (out_dir, target, zig_target) in [
        (linux_out, "x86_64-unknown-linux-gnu", "x86_64-linux-gnu.2.39"),
        (arm_out, "aarch64-unknown-linux-gnu", "aarch64-linux-gnu.2.39"),
    ] {
        let build = plan(out_dir, target, None);
        assert_eq!(after(&build.args, "--prefix"), out_dir.join("zig-out"));
        assert_eq!(after(&build.args, "--cache-dir"), out_dir.join("zig-cache"));
        // the one shared directory: Zig's global cache, from the stamp, never the user's own
        assert_eq!(build.env, [global_cache()]);
        assert_eq!(build.archive, out_dir.join("zig-out/lib/libghostty-vt.a"));
        assert_eq!(build.link_dir, out_dir.join("zig-out/lib"));
        assert_eq!(build.link_name, "ghostty-vt");
        assert_eq!(build.zig, Path::new("/vendor/zig-x86_64-linux-0.17.0/zig"));
        assert_eq!(build.source, Path::new("/vendor/ghostty-vt-0123456789abcdef/src"));
        assert_eq!(build.args[0], "build");
        let flags: Vec<String> = build
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .filter(|arg| arg.starts_with('-'))
            .collect();
        let expected = [
            "--prefix".to_owned(),
            "--cache-dir".to_owned(),
            "-Demit-lib-vt=true".to_owned(),
            "-Demit-xcframework=false".to_owned(),
            "-Doptimize=fast".to_owned(),
            "-Dsimd=true".to_owned(),
            "-Dcpu=baseline".to_owned(),
            format!("-Dtarget={zig_target}"),
            "-Dversion-string=1.4.0".to_owned(),
            "-j10".to_owned(),
        ];
        assert_eq!(flags, expected);
    }
}

#[test]
fn apple_links_a_copy_of_the_archive_alone_and_msvc_its_static_archive() {
    let out_dir = Path::new("/out");
    let apple = plan(out_dir, "aarch64-apple-darwin", Some("14.0"));
    assert_eq!(apple.archive, out_dir.join("zig-out/lib/libghostty-vt.a"));
    assert_eq!(apple.link_dir, out_dir.join("lib"));
    assert_eq!(apple.link_name, "ghostty-vt");
    assert!(apple.args.contains(&OsString::from("-Dtarget=aarch64-macos.14.0")));
    let msvc = plan(out_dir, "x86_64-pc-windows-msvc", None);
    assert_eq!(msvc.archive, out_dir.join("zig-out/lib/ghostty-vt-static.lib"));
    assert_eq!(msvc.link_dir, out_dir.join("zig-out/lib"));
    assert_eq!(msvc.link_name, "ghostty-vt-static");
}

#[test]
fn the_stamp_names_the_pinned_zig_and_its_cache() {
    let text = "# comment\nzig=/vendor/zig\nzig-version=0.17.0\nzig-global-cache=/vendor/zig-cache-0.17.0\n\
                version-string=1.4.0\npatch=0001-x.patch 00ff\n";
    let stamp = Stamp::parse(text).unwrap();
    assert_eq!(stamp.zig, Path::new("/vendor/zig"));
    assert_eq!(stamp.zig_version, "0.17.0");
    assert_eq!(stamp.global_cache, Path::new("/vendor/zig-cache-0.17.0"));
    assert_eq!(stamp.version_string, "1.4.0");
    // `zig` is not read from `zig-version=` or `zig-global-cache=`
    let missing = Stamp::parse("zig-version=0.17.0\nzig-global-cache=/c\nversion-string=1\n").unwrap_err();
    assert!(missing.contains("no zig"), "{missing}");
    let empty = Stamp::parse("zig=\nzig-version=0.17.0\nzig-global-cache=/c\nversion-string=1\n").unwrap_err();
    assert!(empty.contains("no zig"), "{empty}");
}

#[test]
fn the_pin_is_the_revision_and_a_digest_of_the_patches() {
    let pins = "# pins\nLIBGHOSTTY_VT_ZIG_VERSION=\"0.17.0\"\nLIBGHOSTTY_VT_REV=\"83fb9562\"\n";
    let stamp = "zig=/z\nversion-string=1.4.0\npatch=0001-a.patch 00ff\npatch=0002-b.patch 11ee\n";
    let pin = zig_build::pin(pins, stamp).unwrap();
    let (rev, digest) = pin.split_once(' ').unwrap();
    assert_eq!(rev, "83fb9562");
    assert_eq!(digest.len(), 16);
    // the same patches give the same pin; another patch, another order or another revision a different one
    assert_eq!(zig_build::pin(pins, stamp).unwrap(), pin);
    let other_patch = stamp.replace("11ee", "11ef");
    assert_ne!(zig_build::pin(pins, &other_patch).unwrap(), pin);
    let reordered = "patch=0002-b.patch 11ee\npatch=0001-a.patch 00ff\n";
    assert_ne!(zig_build::pin(pins, reordered).unwrap(), pin);
    let other_rev = pins.replace("83fb9562", "83fb9563");
    assert_ne!(zig_build::pin(&other_rev, stamp).unwrap(), pin);
    // the stamp's other lines do not count
    assert_eq!(zig_build::pin(pins, &stamp.replace("1.4.0", "9")).unwrap(), pin);
    let missing = zig_build::pin("LIBGHOSTTY_VT_ZIG_VERSION=\"0.17.0\"\n", stamp).unwrap_err();
    assert!(missing.contains("LIBGHOSTTY_VT_REV"), "{missing}");
}

#[test]
fn the_built_library_carries_the_checked_out_pin() {
    let pins = fs::read_to_string(repo_root().join("build/pins.env")).unwrap();
    let rev = pins
        .lines()
        .find_map(|line| line.strip_prefix("LIBGHOSTTY_VT_REV="))
        .unwrap()
        .trim_matches('"');
    assert!(
        ngterm_ghostty_vt_sys::PIN.starts_with(&format!("{rev} ")),
        "{}",
        ngterm_ghostty_vt_sys::PIN
    );
}

#[test]
fn a_missing_or_unprepared_prefix_names_the_script() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("ghostty-vt-0123456789abcdef");
    let error = zig_build::prepared(&missing).unwrap_err();
    assert!(
        error.starts_with(&format!("no libghostty-vt at {}: ", missing.display())),
        "{error}"
    );
    assert!(error.contains("run scripts/build-ghostty-vt.sh"), "{error}");
    // the prefix of the old layout (a prebuilt library, no tree) is not prepared either
    fs::create_dir_all(missing.join("lib")).unwrap();
    fs::create_dir_all(missing.join("include/ghostty")).unwrap();
    fs::write(missing.join("lib/libghostty-vt.a"), "!<arch>\n").unwrap();
    fs::write(missing.join("include/ghostty/vt.h"), "").unwrap();
    assert!(
        zig_build::prepared(&missing)
            .unwrap_err()
            .contains("run scripts/build-ghostty-vt.sh")
    );
    fs::create_dir_all(missing.join("src")).unwrap();
    fs::create_dir_all(missing.join("share/ngterm")).unwrap();
    fs::write(missing.join("src/build.zig"), "").unwrap();
    fs::write(
        missing.join(zig_build::STAMP),
        "zig=/z\nzig-version=0.17.0\nzig-global-cache=/c\nversion-string=1.4.0\n",
    )
    .unwrap();
    assert_eq!(zig_build::prepared(&missing).unwrap().zig, Path::new("/z"));
}

#[test]
fn a_pin_or_patch_change_reruns_the_build_script() {
    // with the prefix keyed by both (prefix_keys.rs), the rerun finds the new prefix and builds its tree
    for input in ["build/pins.env", "build/patches/ghostty-vt"] {
        assert!(zig_build::RERUN_FILES.contains(&input), "{input}");
    }
    for input in zig_build::RERUN_FILES {
        assert!(repo_root().join(input).exists(), "{input}");
    }
    for name in [
        script::PREFIX_ENV,
        "NGTERM_VENDOR",
        "HOME",
        "MACOSX_DEPLOYMENT_TARGET",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "ANDROID_PLATFORM",
        "ANDROID_NDK_HOME",
        "CARGO_NDK_SYSROOT_PATH",
    ] {
        assert!(zig_build::RERUN_ENV.contains(&name), "{name}");
    }
}

/// Git Bash is found beside the `git.exe` of `PATH` (Git's `cmd\`, `bin\` and `mingw64\bin\`), then under the install
/// locations, and only ever as Git's own `bin\bash.exe`: a `PATH` entry holding a `bash.exe` but no `git.exe`
/// (WSL's `System32`) is never a candidate.
#[test]
fn git_bash_is_found_beside_git_and_never_from_system32() {
    let root = Path::new("/drive");
    let git = root.join("Git");
    let path = std::env::join_paths([
        root.join("Windows").join("System32"),
        git.join("cmd"),
        root.join("Portable").join("mingw64").join("bin"),
        root.join("Tools").join("bin"),
    ])
    .unwrap();
    let exists = |file: &Path| {
        file == git.join("cmd").join("git.exe")
            || file == root.join("Portable").join("mingw64").join("bin").join("git.exe")
            || file == root.join("Windows").join("System32").join("bash.exe")
    };
    let var = |name: &str| match name {
        "PATH" => Some(path.clone()),
        "ProgramFiles" | "ProgramW6432" => Some(root.join("Program Files").into_os_string()),
        "LOCALAPPDATA" => Some(
            root.join("Users")
                .join("me")
                .join("AppData")
                .join("Local")
                .into_os_string(),
        ),
        _ => None,
    };
    let bash = |dir: PathBuf| dir.join("bin").join("bash.exe");
    assert_eq!(
        script::git_bash_candidates(&var, &exists),
        [
            bash(git.clone()),
            bash(root.join("Portable")),
            bash(root.join("Program Files").join("Git")),
            bash(
                root.join("Users")
                    .join("me")
                    .join("AppData")
                    .join("Local")
                    .join("Programs")
                    .join("Git")
            ),
        ]
    );
    assert!(script::git_bash_candidates(&|_| None, &exists).is_empty());
}

#[test]
fn bash_takes_forward_slashes_on_windows() {
    let path = Path::new("C:\\repo\\scripts/build-ghostty-vt.sh");
    let expected = if cfg!(windows) {
        "C:/repo/scripts/build-ghostty-vt.sh"
    } else {
        "C:\\repo\\scripts/build-ghostty-vt.sh"
    };
    assert_eq!(script::bash_arg(path), expected);
}

#[test]
fn the_prepare_script_fetches_the_packages_of_every_mapped_target() {
    let script = fs::read_to_string(repo_root().join("scripts/build-ghostty-vt.sh")).unwrap();
    let start = script.find("\nZIG_TARGETS=(").unwrap() + "\nZIG_TARGETS=(".len();
    let end = start + script[start..].find(')').unwrap();
    let fetched: Vec<&str> = script[start..end].split_whitespace().collect();
    let mapped: Vec<&str> = zig_build::ZIG_TARGETS.iter().map(|(_, zig)| *zig).collect();
    assert_eq!(fetched, mapped);
}

/// The real build script against an empty vendor root: the build fails with the error that names the prepare script.
#[test]
fn a_build_without_its_prefix_fails_naming_the_script() {
    let vendor = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO"))
        .args([
            "build",
            "--offline",
            "--locked",
            "-p",
            "ngterm-ghostty-vt-sys",
            "--target-dir",
        ])
        .arg(target_dir.path())
        .current_dir(repo_root())
        .env("NGTERM_VENDOR", vendor.path())
        .env_remove(script::PREFIX_ENV)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    // on Windows the prefix is cygpath's mixed form of the vendor root (`C:/...`)
    let vendor_root = vendor.path().display().to_string();
    let vendor_root = if cfg!(windows) {
        vendor_root.replace('\\', "/")
    } else {
        vendor_root
    };
    assert!(
        stderr.contains(&format!("no libghostty-vt at {vendor_root}/ghostty-vt-")),
        "{stderr}"
    );
    assert!(stderr.contains("run scripts/build-ghostty-vt.sh"), "{stderr}");
}

/// The prefix the build script built from before this test ran (`NGTERM_GHOSTTY_VT_PREFIX`, else the script's).
fn prefix() -> PathBuf {
    script::prefix(&repo_root()).unwrap()
}

fn entries(dir: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

/// Whether this host configures Zig triple `zig`: any but an iOS one, which needs Xcode's SDK of it on a macOS host
/// (`ios_sdk_present` in `scripts/build-ghostty-vt.sh`).
fn configures_here(zig: &str) -> bool {
    let sdk = if zig.ends_with("-ios-simulator") {
        "iphonesimulator"
    } else if zig.ends_with("-ios") {
        "iphoneos"
    } else {
        return true;
    };
    cfg!(target_os = "macos")
        && Command::new("xcrun")
            .args(["--sdk", sdk, "--show-sdk-path"])
            .output()
            .is_ok_and(|output| output.status.success())
}

/// A prepared prefix needs no network, for any mapped target this host configures (iOS ones only on a Mac with Xcode's
/// iOS SDK): Zig fetches packages only while it configures a build, so each target's libghostty-vt configuration is
/// run with every proxy pointing at a closed port and an empty global cache (which would otherwise serve a package the
/// tree lacks), and must find every package in the tree, which it leaves as it was.
#[test]
fn the_prepared_prefix_configures_every_target_with_networking_off() {
    const CLOSED: &str = "http://127.0.0.1:9";
    let prefix = prefix();
    let stamp = zig_build::prepared(&prefix).unwrap();
    let packages = prefix.join(zig_build::SOURCE).join("zig-pkg");
    let before = entries(&packages);
    let scratch = tempfile::tempdir().unwrap();
    for (cargo, zig) in zig_build::ZIG_TARGETS {
        if !configures_here(zig) {
            continue;
        }
        let target = zig_build::zig_target(cargo, None, None, None).unwrap();
        let mut command = Command::new(&stamp.zig);
        // an Android configuration only joins paths into the NDK, which must exist (build-ghostty-vt.sh)
        command.env("ANDROID_NDK_HOME", scratch.path());
        command
            .args(["build", "--list-steps", "-Demit-lib-vt=true"])
            .arg(format!("-Dtarget={target}"))
            .arg("--cache-dir")
            .arg(scratch.path().join("local"))
            .env("ZIG_GLOBAL_CACHE_DIR", scratch.path().join("global"))
            .current_dir(prefix.join(zig_build::SOURCE));
        for proxy in [
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
        ] {
            command.env(proxy, CLOSED);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(entries(&packages), before);
}
