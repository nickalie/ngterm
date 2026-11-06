//! `scripts/ios-privacy-check.sh`, the scan behind the iOS app's privacy manifest: it passes only when the bundle's
//! `PrivacyInfo.xcprivacy` declares no tracking and no collected data, declares the category of every required-reason
//! API the executable uses (an import, or an Objective-C selector), with reasons Apple approves for it, and declares no
//! category the executable doesn't use.
//!
//! Every case builds a small app bundle around a real Mach-O executable compiled here from C (`cc`), which imports the
//! APIs the case names and holds the selectors it names in `__objc_methname`, so the script reads it with the real
//! `nm`, `otool` and `plutil`. macOS hosts only: those tools are; `scripts/build-ios.sh archive` runs the script on the
//! real Release archive.
#![cfg(target_os = "macos")]

/// The case's app bundle, its manifest and the run of the script.
#[cfg(test)]
mod bundle {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    pub(super) fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// What the case's executable uses.
    pub(super) struct Uses {
        /// C functions of `<sys/stat.h>`, `<sys/attr.h>` and `<sys/mount.h>`, called with dummy arguments.
        pub(super) calls: &'static [&'static str],
        /// Objective-C method names, as the compiler would emit them.
        pub(super) selectors: &'static [&'static str],
    }

    /// The C call that imports `function`.
    pub(super) fn call(function: &str) -> &'static str {
        match function {
            "fstat" => "struct stat s; r += fstat(argc, &s);",
            "getattrlist" => "struct attrlist a = {0}; char b[64]; r += getattrlist(argv[0], &a, b, sizeof b, 0);",
            "statfs" => "struct statfs f; r += statfs(argv[0], &f);",
            other => panic!("no C call for {other}"),
        }
    }

    /// A manifest: whether it says it tracks, and each declared category (the `NSPrivacyAccessedAPICategory` suffix)
    /// with its reasons.
    pub(super) struct Manifest {
        pub(super) tracking: bool,
        pub(super) types: &'static [(&'static str, &'static [&'static str])],
    }

    impl Manifest {
        pub(super) fn xml(&self) -> String {
            let mut types = String::new();
            for (category, reasons) in self.types {
                let reasons = reasons.join("</string><string>");
                write!(
                    types,
                    "<dict><key>NSPrivacyAccessedAPIType</key><string>NSPrivacyAccessedAPICategory{category}</string>\
                     <key>NSPrivacyAccessedAPITypeReasons</key><array><string>{reasons}</string></array></dict>"
                )
                .unwrap();
            }
            let tracking = if self.tracking { "<true/>" } else { "<false/>" };
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>NSPrivacyTracking</key>{tracking}<key>NSPrivacyTrackingDomains</key><array/>\
                 <key>NSPrivacyCollectedDataTypes</key><array/>\
                 <key>NSPrivacyAccessedAPITypes</key><array>{types}</array></dict></plist>\n"
            )
        }
    }

    /// Builds `App.app` in `dir` (its manifest left out when `manifest` is `None`), and runs the script on it.
    pub(super) fn check(dir: &Path, uses: &Uses, manifest: Option<&Manifest>) -> Output {
        let app = dir.join("App.app");
        fs::create_dir_all(&app).unwrap();
        let calls: String = uses.calls.iter().map(|function| call(function)).collect();
        let mut selectors = String::new();
        for (i, selector) in uses.selectors.iter().enumerate() {
            writeln!(
                selectors,
                "__attribute__((used, section(\"__TEXT,__objc_methname,cstring_literals\"))) \
                 static const char selector{i}[] = \"{selector}\";"
            )
            .unwrap();
        }
        let source = format!(
            "#include <sys/attr.h>\n#include <sys/mount.h>\n#include <sys/stat.h>\n#include <unistd.h>\n{selectors}\
             int main(int argc, char **argv) {{ int r = 0; {calls} return r; }}\n"
        );
        fs::write(dir.join("main.c"), source).unwrap();
        let compiled = Command::new("cc")
            .arg("-o")
            .arg(app.join("App"))
            .arg(dir.join("main.c"))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "cc: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        fs::write(
            app.join("Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleExecutable</key><string>App</string></dict></plist>\n",
        )
        .unwrap();
        if let Some(manifest) = manifest {
            fs::write(app.join("PrivacyInfo.xcprivacy"), manifest.xml()).unwrap();
        }
        Command::new(repo_root().join("scripts/ios-privacy-check.sh"))
            .arg(&app)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    pub(super) fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The app's own use: `fstat` from the core's libraries and `systemUptime` from the terminal.
    pub(super) const APP_USES: Uses = Uses {
        calls: &["fstat"],
        selectors: &["systemUptime"],
    };

    pub(super) const APP_MANIFEST: Manifest = Manifest {
        tracking: false,
        types: &[("FileTimestamp", &["C617.1"]), ("SystemBootTime", &["35F9.1"])],
    };
}

use bundle::{APP_MANIFEST, APP_USES, Manifest, Uses, check, stderr};

#[test]
fn a_manifest_declaring_every_used_category_passes() {
    let dir = tempfile::tempdir().unwrap();
    let output = check(dir.path(), &APP_USES, Some(&APP_MANIFEST));
    assert!(output.status.success(), "{}", stderr(&output));
    let said = stderr(&output);
    assert!(said.contains("FileTimestamp: fstat"), "{said}");
    assert!(said.contains("SystemBootTime: systemUptime"), "{said}");
}

#[test]
fn an_undeclared_category_fails_naming_the_api() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        tracking: false,
        types: &[("FileTimestamp", &["C617.1"])],
    };
    let output = check(dir.path(), &APP_USES, Some(&manifest));
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("uses systemUptime: declare NSPrivacyAccessedAPICategorySystemBootTime"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_category_the_executable_doesnt_use_fails() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        tracking: false,
        types: &[
            ("FileTimestamp", &["C617.1"]),
            ("SystemBootTime", &["35F9.1"]),
            ("DiskSpace", &["E174.1"]),
        ],
    };
    let output = check(dir.path(), &APP_USES, Some(&manifest));
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("NSPrivacyAccessedAPICategoryDiskSpace is declared, but App uses none of its APIs"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_reason_of_another_category_fails() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        tracking: false,
        types: &[("FileTimestamp", &["C617.1"]), ("SystemBootTime", &["C617.1"])],
    };
    let output = check(dir.path(), &APP_USES, Some(&manifest));
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("NSPrivacyAccessedAPICategorySystemBootTime: 'C617.1' is not a reason an app may"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn tracking_fails() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        tracking: true,
        types: APP_MANIFEST.types,
    };
    let output = check(dir.path(), &APP_USES, Some(&manifest));
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("NSPrivacyTracking is not false"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_bundle_without_a_manifest_fails() {
    let dir = tempfile::tempdir().unwrap();
    let output = check(dir.path(), &APP_USES, None);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("has no PrivacyInfo.xcprivacy"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_api_of_two_categories_is_covered_by_either() {
    let dir = tempfile::tempdir().unwrap();
    let uses = Uses {
        calls: &["getattrlist", "statfs"],
        selectors: &[],
    };
    let manifest = Manifest {
        tracking: false,
        types: &[("DiskSpace", &["E174.1"])],
    };
    let output = check(dir.path(), &uses, Some(&manifest));
    assert!(output.status.success(), "{}", stderr(&output));
}
