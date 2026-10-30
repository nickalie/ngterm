//! herdr: the `default_windows_pane_shell` and image tests of `src/pane.rs` at `HERDR_REV`, licensed under the Apache
//! License 2.0, modified: each test owns a temporary directory; the hosts are named.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;

/// A structurally valid PE32+ image whose fields tests make malformed one way at a time.
fn image(machine: u16, characteristics: u16, sections: u16) -> Vec<u8> {
    const PE_OFFSET: u32 = 0x80;
    const OPTIONAL_HEADER_LEN: u16 = 0x70;
    let pe = PE_OFFSET as usize;
    let mut bytes = vec![0u8; pe + PE_HEADERS_LEN + usize::from(OPTIONAL_HEADER_LEN) + usize::from(sections) * 40];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[PE_OFFSET_FIELD..PE_OFFSET_FIELD + 4].copy_from_slice(&PE_OFFSET.to_le_bytes());
    bytes[pe..pe + 4].copy_from_slice(PE_SIGNATURE);
    let coff = pe + 4;
    bytes[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
    bytes[coff + 2..coff + 4].copy_from_slice(&sections.to_le_bytes());
    bytes[coff + 16..coff + 18].copy_from_slice(&OPTIONAL_HEADER_LEN.to_le_bytes());
    bytes[coff + 18..coff + 20].copy_from_slice(&characteristics.to_le_bytes());
    bytes[pe + PE_HEADERS_LEN..pe + PE_HEADERS_LEN + 2].copy_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
    bytes
}

fn x64_image() -> Vec<u8> {
    image(MACHINE_AMD64, IMAGE_FILE_EXECUTABLE_IMAGE, 1)
}

fn path_of(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs).unwrap()
}

/// The shell an x64 host without a system directory picks under `PATH` = `dirs`.
fn shell_under(dirs: &[&Path]) -> Option<String> {
    shell_on_path(Some(&path_of(dirs)), None, MACHINE_AMD64)
}

/// What the search returns for `path`: its full path, as a string.
fn found(path: PathBuf) -> Option<String> {
    path.into_os_string().into_string().ok()
}

/// The inbox PowerShell's place under the system directory `system`, with a valid image in it.
fn inbox_under(system: &Path) -> PathBuf {
    let dir = system.join("WindowsPowerShell").join("v1.0");
    fs::create_dir_all(&dir).unwrap();
    let inbox = dir.join(INBOX_POWERSHELL);
    fs::write(&inbox, x64_image()).unwrap();
    inbox
}

#[test]
fn pwsh_on_path_is_preferred_as_a_full_path() {
    let dir = tempfile::tempdir().unwrap();
    let pwsh = dir.path().join(PWSH);
    fs::write(&pwsh, x64_image()).unwrap();
    assert_eq!(shell_under(&[dir.path()]), found(pwsh));
}

#[test]
fn pwsh_anywhere_on_path_beats_powershell_and_powershell_beats_the_system_directory() {
    let base = tempfile::tempdir().unwrap();
    let (first, second, system) = (
        base.path().join("first"),
        base.path().join("second"),
        base.path().join("sys"),
    );
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let inbox = inbox_under(&system);
    let search = |dirs: &[&Path]| shell_on_path(Some(&path_of(dirs)), Some(&system), MACHINE_AMD64);
    // nothing on PATH: the system directory's
    assert_eq!(search(&[&first, &second]), found(inbox.clone()));
    // a powershell.exe on PATH comes first
    let powershell = first.join(INBOX_POWERSHELL);
    fs::write(&powershell, x64_image()).unwrap();
    assert_eq!(search(&[&first, &second]), found(powershell));
    // a pwsh.exe in a later directory beats it
    let pwsh = second.join(PWSH);
    fs::write(&pwsh, x64_image()).unwrap();
    assert_eq!(search(&[&first, &second]), found(pwsh));
    // a relative system directory is never searched
    assert_eq!(
        shell_on_path(None, Some(Path::new("sys")), MACHINE_AMD64),
        None,
        "relative system directory"
    );
    assert_eq!(shell_on_path(None, Some(&system), MACHINE_AMD64), found(inbox));
}

#[test]
fn without_a_launchable_powershell_there_is_no_shell() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(PWSH), b"not a PE image").unwrap();
    fs::write(dir.path().join(INBOX_POWERSHELL), b"MZ").unwrap();
    assert_eq!(shell_under(&[dir.path()]), None);
    assert_eq!(shell_on_path(None, None, MACHINE_AMD64), None);
    // an empty PATH, and a relative directory that would resolve against the working directory
    assert_eq!(shell_on_path(Some(OsStr::new("")), None, MACHINE_AMD64), None);
    assert_eq!(shell_under(&[Path::new("relative")]), None);
    // a system directory without the inbox PowerShell
    let system = tempfile::tempdir().unwrap();
    assert_eq!(shell_on_path(None, Some(system.path()), MACHINE_AMD64), None);
}

#[test]
fn malformed_or_foreign_images_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let pwsh = dir.path().join(PWSH);
    let truncated_header = {
        let mut bytes = b"MZ".to_vec();
        bytes.resize(DOS_HEADER_LEN, 0);
        bytes[PE_OFFSET_FIELD..PE_OFFSET_FIELD + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes
    };
    // one declared section, but the file ends before its 40-byte header
    let truncated_section_table = {
        let mut bytes = x64_image();
        bytes.truncate(bytes.len() - 1);
        bytes
    };
    let wrong_magic = {
        let mut bytes = x64_image();
        let at = 0x80 + PE_HEADERS_LEN;
        bytes[at..at + 2].copy_from_slice(&0x0107u16.to_le_bytes());
        bytes
    };
    let cases: [(&str, Vec<u8>); 9] = [
        ("empty", Vec::new()),
        ("dos signature only", b"MZ".to_vec()),
        ("truncated pe header", truncated_header),
        ("truncated section table", truncated_section_table),
        (
            "dll",
            image(MACHINE_AMD64, IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_DLL, 1),
        ),
        ("no executable bit", image(MACHINE_AMD64, 0, 1)),
        ("foreign machine", image(0x0200, IMAGE_FILE_EXECUTABLE_IMAGE, 1)),
        ("zero sections", image(MACHINE_AMD64, IMAGE_FILE_EXECUTABLE_IMAGE, 0)),
        ("not PE32 or PE32+", wrong_magic),
    ];
    for (label, bytes) in cases {
        fs::write(&pwsh, &bytes).unwrap();
        assert_eq!(shell_under(&[dir.path()]), None, "case {label:?}");
    }
    fs::write(&pwsh, x64_image()).unwrap();
    assert_eq!(shell_under(&[dir.path()]), found(pwsh), "the valid image");
}

#[test]
fn machine_compatibility_tracks_the_native_host() {
    assert!(machine_is_compatible(MACHINE_I386, MACHINE_AMD64));
    assert!(machine_is_compatible(MACHINE_AMD64, MACHINE_AMD64));
    assert!(!machine_is_compatible(MACHINE_ARM64, MACHINE_AMD64));
    assert!(machine_is_compatible(MACHINE_I386, MACHINE_ARM64));
    assert!(machine_is_compatible(MACHINE_AMD64, MACHINE_ARM64));
    assert!(machine_is_compatible(MACHINE_ARM64, MACHINE_ARM64));
    assert!(machine_is_compatible(MACHINE_I386, MACHINE_I386));
    assert!(!machine_is_compatible(MACHINE_AMD64, MACHINE_I386));
    assert!(!machine_is_compatible(0x0200, MACHINE_AMD64));
    // an unknown host counts as x64
    assert!(machine_is_compatible(MACHINE_AMD64, 0));
    assert!(!machine_is_compatible(MACHINE_ARM64, 0));
}

#[test]
fn an_arm64_image_launches_only_on_an_arm64_host() {
    let dir = tempfile::tempdir().unwrap();
    let pwsh = dir.path().join(PWSH);
    fs::write(&pwsh, image(MACHINE_ARM64, IMAGE_FILE_EXECUTABLE_IMAGE, 1)).unwrap();
    assert!(
        !is_launchable(&pwsh, MACHINE_AMD64),
        "x64 Windows cannot launch a pure ARM64 image"
    );
    assert!(is_launchable(&pwsh, MACHINE_ARM64));
    assert!(!is_launchable(&pwsh, MACHINE_I386));
}

#[test]
fn invalid_candidates_are_skipped_for_a_later_directory() {
    let base = tempfile::tempdir().unwrap();
    let (invalid, valid) = (base.path().join("invalid"), base.path().join("valid"));
    fs::create_dir_all(&invalid).unwrap();
    fs::create_dir_all(&valid).unwrap();
    fs::write(invalid.join(PWSH), b"MZ").unwrap();
    let pwsh = valid.join(PWSH);
    fs::write(&pwsh, x64_image()).unwrap();
    // a directory named pwsh.exe is no candidate either
    let directory = base.path().join("directory");
    fs::create_dir_all(directory.join(PWSH)).unwrap();
    assert_eq!(shell_under(&[&directory, &invalid, &valid]), found(pwsh));
}

/// The package an MSIX PowerShell 7 installs, as its alias names it.
const PACKAGE: &str = "Microsoft.PowerShell_8wekyb3d8bbwe";
const APP_ID: &str = "Microsoft.PowerShell_8wekyb3d8bbwe!App";
const PACKAGED_PWSH: &str = r"C:\Program Files\WindowsApps\Microsoft.PowerShell_7.5.4.0_x64__8wekyb3d8bbwe\pwsh.exe";

/// An App Execution Alias's reparse buffer with `version` and `strings`, each NUL-terminated, laid out as Windows
/// writes one (`fsutil reparsepoint query` on a Windows 11 alias): the header, the version, the UTF-16 strings.
fn alias_buffer_of(version: u32, strings: &[&str]) -> Vec<u8> {
    let mut data = version.to_le_bytes().to_vec();
    for string in strings {
        data.extend(string.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    }
    let mut buffer = IO_REPARSE_TAG_APPEXECLINK.to_le_bytes().to_vec();
    buffer.extend(u16::try_from(data.len()).unwrap().to_le_bytes());
    buffer.extend([0, 0]);
    buffer.extend(data);
    buffer
}

/// The reparse buffer of an alias of PowerShell 7's package whose image is `target`.
fn alias_buffer(target: &str) -> Vec<u8> {
    alias_buffer_of(APP_EXEC_LINK_VERSION, &[PACKAGE, APP_ID, target, "0"])
}

#[test]
fn an_app_execution_alias_names_its_packaged_image() {
    assert_eq!(
        app_exec_link_target(&alias_buffer(PACKAGED_PWSH)).as_deref(),
        Some(PACKAGED_PWSH)
    );
    // an older alias without the application type still names its image
    let three = alias_buffer_of(APP_EXEC_LINK_VERSION, &[PACKAGE, APP_ID, PACKAGED_PWSH]);
    assert_eq!(app_exec_link_target(&three).as_deref(), Some(PACKAGED_PWSH));
    // data past what the header declares is not read
    let mut trailing = alias_buffer(PACKAGED_PWSH);
    trailing.extend([0xff; 6]);
    assert_eq!(app_exec_link_target(&trailing).as_deref(), Some(PACKAGED_PWSH));
}

#[test]
fn other_reparse_buffers_name_no_image() {
    let other_tag = {
        let mut buffer = alias_buffer(PACKAGED_PWSH);
        // IO_REPARSE_TAG_SYMLINK
        buffer[..4].copy_from_slice(&0xa000_000c_u32.to_le_bytes());
        buffer
    };
    let truncated = {
        let mut buffer = alias_buffer(PACKAGED_PWSH);
        buffer.pop();
        buffer
    };
    let odd = {
        let mut buffer = alias_buffer(PACKAGED_PWSH);
        buffer.push(0);
        let len = u16::from_le_bytes([buffer[4], buffer[5]]) + 1;
        buffer[4..6].copy_from_slice(&len.to_le_bytes());
        buffer
    };
    let unterminated = {
        let mut buffer = alias_buffer_of(APP_EXEC_LINK_VERSION, &[PACKAGE, APP_ID]);
        let path: Vec<u8> = PACKAGED_PWSH.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let len = u16::from_le_bytes([buffer[4], buffer[5]]) + u16::try_from(path.len()).unwrap();
        buffer[4..6].copy_from_slice(&len.to_le_bytes());
        buffer.extend(path);
        buffer
    };
    let cases: [(&str, Vec<u8>); 9] = [
        ("empty", Vec::new()),
        (
            "header only",
            alias_buffer_of(APP_EXEC_LINK_VERSION, &[])[..REPARSE_HEADER_LEN].to_vec(),
        ),
        ("another tag", other_tag),
        (
            "another version",
            alias_buffer_of(2, &[PACKAGE, APP_ID, PACKAGED_PWSH, "0"]),
        ),
        ("shorter than its header says", truncated),
        ("half a UTF-16 unit", odd),
        ("no image", alias_buffer_of(APP_EXEC_LINK_VERSION, &[PACKAGE, APP_ID])),
        ("an empty image", alias_buffer("")),
        ("an unterminated image", unterminated),
    ];
    for (label, buffer) in cases {
        assert_eq!(app_exec_link_target(&buffer), None, "case {label:?}");
    }
}

/// Makes `alias` an App Execution Alias of PowerShell 7's package whose image is `target`, as an MSIX install does.
#[cfg(windows)]
fn make_alias(alias: &Path, target: &str) {
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(alias)
        .unwrap();
    let buffer = alias_buffer(target);
    let mut returned = 0u32;
    // SAFETY: the handle is open for writing, the input buffer is valid for the length passed beside it, there is no
    // output, and the call is synchronous.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            u32::try_from(buffer.len()).unwrap(),
            std::ptr::null_mut(),
            0,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(ok, 0, "setting the alias: {}", std::io::Error::last_os_error());
}

#[cfg(windows)]
#[test]
fn an_alias_of_a_launchable_image_is_the_shell_itself() {
    let base = tempfile::tempdir().unwrap();
    let (apps, package) = (base.path().join("WindowsApps"), base.path().join("package"));
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(&package).unwrap();
    let image = package.join(PWSH);
    fs::write(&image, x64_image()).unwrap();
    let alias = apps.join(PWSH);
    make_alias(&alias, image.to_str().unwrap());
    // the alias is no image: opening it without its reparse point fails, as on a real one
    assert!(fs::File::open(&alias).is_err(), "the alias opens like a file");
    assert_eq!(alias_target(&alias), Some(image));
    assert_eq!(shell_under(&[&apps]), found(alias), "the alias, not its image");
}

#[cfg(windows)]
#[test]
fn an_alias_of_no_launchable_image_is_skipped() {
    let base = tempfile::tempdir().unwrap();
    let (apps, package, later) = (
        base.path().join("WindowsApps"),
        base.path().join("package"),
        base.path().join("later"),
    );
    for dir in [&apps, &package, &later] {
        fs::create_dir_all(dir).unwrap();
    }
    let arm64 = package.join("arm64.exe");
    fs::write(&arm64, image(MACHINE_ARM64, IMAGE_FILE_EXECUTABLE_IMAGE, 1)).unwrap();
    let not_an_image = package.join("text.exe");
    fs::write(&not_an_image, b"not a PE image").unwrap();
    let missing = package.join("missing.exe");
    let pwsh = later.join(PWSH);
    fs::write(&pwsh, x64_image()).unwrap();
    for (label, target) in [
        ("a foreign image", arm64.to_str().unwrap()),
        ("not an image", not_an_image.to_str().unwrap()),
        ("a missing image", missing.to_str().unwrap()),
        ("a relative image", r"package\pwsh.exe"),
    ] {
        let alias = apps.join(PWSH);
        make_alias(&alias, target);
        assert!(!is_launchable(&alias, MACHINE_AMD64), "case {label:?}");
        assert_eq!(shell_under(&[&apps, &later]), found(pwsh.clone()), "case {label:?}");
        fs::remove_file(&alias).unwrap();
    }
}
