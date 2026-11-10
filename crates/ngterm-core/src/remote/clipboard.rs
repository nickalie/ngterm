//! Image clipboard transfers to remote origins. The image travels on SSH stdin, outside the terminal PTY, and is
//! retained in a private remote temporary directory so a coding agent can read it after the paste: on a POSIX origin
//! `/tmp`, on a Windows one the account's `%TEMP%` (`ngtermctl origin-run`, [`super::origin`]).

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::origin::{self, OriginRun, OriginShell};
use super::tree::{is_host, ssh_arguments};
use crate::command_restore::shell_quoted_line;

const DEADLINE: Duration = Duration::from_secs(60);
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Upload a clipboard image and return its absolute filename on the origin. Blocks; run off the UI thread.
///
/// # Errors
///
/// Invalid destination or format, empty or oversized input, an SSH failure or deadline, or an invalid reply.
pub fn upload(host: &str, extension: &str, bytes: &[u8]) -> Result<String, String> {
    let shell = origin::shell_of(host);
    let argv = upload_command(host, extension, shell)?;
    transfer(&argv, bytes, DEADLINE, shell).map_err(|error| error.to_string())
}

fn upload_command(host: &str, extension: &str, shell: OriginShell) -> Result<Vec<String>, String> {
    if !is_host(host) || !extension.chars().all(|ch| ch.is_ascii_alphanumeric()) || extension.is_empty() {
        return Err("Invalid image upload destination or format".to_owned());
    }
    if shell == OriginShell::Windows {
        let mut argv = ssh_arguments(host, 5, false);
        argv.push(
            OriginRun::Paste {
                extension: extension.to_owned(),
            }
            .tail(),
        );
        return Ok(argv);
    }
    // /tmp is present on the POSIX origins that remote attach supports. A fresh directory avoids following an
    // existing file or symlink; umask protects both it and the image. The shell sees only a fixed format extension.
    let script = format!(
        "umask 077; dir=$(mktemp -d /tmp/ngterm-paste.XXXXXXXXXX) || exit 1; \
         file=\"$dir/clipboard.{extension}\"; \
         trap 'rm -f \"$file\"; rmdir \"$dir\"' 0 1 2 15; \
         cat >\"$file\" || exit 1; trap - 0 1 2 15; printf '%s' \"$file\""
    );
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(shell_quoted_line(&["/bin/sh", "-c", &script]));
    Ok(argv)
}

/// Files instead of pipes bound both the transfer and output capture, including an SSH wrapper retaining stdio.
fn transfer(argv: &[String], bytes: &[u8], deadline: Duration, shell: OriginShell) -> std::io::Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(std::io::Error::other("Clipboard image is empty or exceeds 64 MiB"));
    }
    let mut input = tempfile::tempfile()?;
    input.write_all(bytes)?;
    input.rewind()?;
    let mut output = tempfile::tempfile()?;
    let mut errors = tempfile::tempfile()?;
    let mut child = super::tree::local_command(argv)?
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(errors.try_clone()?))
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < deadline => std::thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result
                    .err()
                    .unwrap_or_else(|| std::io::Error::other("Image upload timed out")));
            }
        }
    };
    if !status.success() {
        errors.seek(SeekFrom::Start(0))?;
        let mut message = String::new();
        errors.take(4096).read_to_string(&mut message)?;
        return Err(std::io::Error::other(format!(
            "Image upload failed: {}",
            message.trim()
        )));
    }
    output.rewind()?;
    let mut path = String::new();
    output.take(4096).read_to_string(&mut path)?;
    // Login banners and unexpected stdout must never turn into terminal input.
    let valid = stored_path_is_valid(&path, shell);
    if !valid {
        return Err(std::io::Error::other("Image upload returned an invalid filename"));
    }
    Ok(path)
}

/// Whether `path` is what the origin's upload prints: a file `clipboard.EXTENSION` in a fresh `ngterm-paste.ID`
/// directory, with nothing else in the line.
fn stored_path_is_valid(path: &str, shell: OriginShell) -> bool {
    let tail = match shell {
        OriginShell::Posix => path.strip_prefix("/tmp/ngterm-paste."),
        // the directory is under the account's %TEMP%, wherever that is, so all it can check of that part is its shape
        OriginShell::Windows if !path.chars().any(char::is_control) => path
            .rsplit_once(r"\ngterm-paste.")
            .filter(|(base, _)| is_absolute_windows_path(base))
            .map(|(_, tail)| tail),
        OriginShell::Windows => None,
    };
    let separator = if shell == OriginShell::Windows {
        r"\clipboard."
    } else {
        "/clipboard."
    };
    tail.and_then(|tail| tail.split_once(separator))
        .is_some_and(|(id, extension)| {
            id.len() == 10
                && id.chars().all(|ch| ch.is_ascii_alphanumeric())
                && !extension.is_empty()
                && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
}

/// Whether `path` reads as an absolute Windows path (a drive's, or a share's) and nothing else: a banner printed ahead
/// of the real path starts with neither, or carries a character a path cannot hold.
fn is_absolute_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let rest = if bytes.len() > 1 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        &path[2..]
    } else if path.starts_with(r"\\") {
        path
    } else {
        return false;
    };
    (rest.is_empty() || rest.starts_with('\\')) && !rest.contains(['"', '<', '>', '|', '*', '?', ':'])
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn a_stored_path_is_only_what_the_origin_prints() {
        for (path, shell, valid) in [
            ("/tmp/ngterm-paste.abcdefghij/clipboard.png", OriginShell::Posix, true),
            ("/tmp/ngterm-paste.abc/clipboard.png", OriginShell::Posix, false),
            (
                r"C:\Users\me\AppData\Local\Temp\ngterm-paste.abcdefghij\clipboard.png",
                OriginShell::Windows,
                true,
            ),
            (r"C:\Temp\ngterm-paste.abc\clipboard.png", OriginShell::Windows, false),
            (
                "C:\\Temp\\ngterm-paste.abcdefghij\\clipboard.png\r\ncommand",
                OriginShell::Windows,
                false,
            ),
            (r"D:\ngterm-paste.abcdefghij\clipboard.png", OriginShell::Windows, true),
            (
                r"\\host\t\ngterm-paste.abcdefghij\clipboard.png",
                OriginShell::Windows,
                true,
            ),
            (
                r"Welcome to the box \ngterm-paste.abcdefghij\clipboard.png",
                OriginShell::Windows,
                false,
            ),
            (
                r"C:\a: note \ngterm-paste.abcdefghij\clipboard.png",
                OriginShell::Windows,
                false,
            ),
            ("banner", OriginShell::Windows, false),
            (
                "/tmp/ngterm-paste.abcdefghij/clipboard.png",
                OriginShell::Windows,
                false,
            ),
        ] {
            assert_eq!(stored_path_is_valid(path, shell), valid, "{path}");
        }
    }

    #[test]
    fn a_windows_origins_upload_is_the_paste_run() {
        let argv = upload_command("example", "png", OriginShell::Windows).unwrap();
        let word = argv.last().unwrap().rsplit(' ').next().unwrap();
        assert_eq!(
            OriginRun::decode(word).unwrap(),
            OriginRun::Paste {
                extension: "png".to_owned()
            }
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn upload_delivers_exact_bytes_to_a_private_remote_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let argv = upload_command("example", "png", OriginShell::Posix).unwrap();
        let command = vec!["/bin/sh".to_owned(), "-c".to_owned(), argv.last().unwrap().clone()];
        let bytes = b"\x89PNG\r\n\x1a\n\0\xffbinary\x1b[201~";
        let path = transfer(&command, bytes, Duration::from_secs(5), OriginShell::Posix).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let directory = std::path::Path::new(&path).parent().unwrap();
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failures_and_unexpected_output_do_not_become_pasted_paths() {
        for script in [
            "exit 1",
            "printf banner",
            "printf '/tmp/ngterm-paste.bad\\ncommand'",
            "sleep 1",
        ] {
            let argv = ["/bin/sh", "-c", script].map(str::to_owned);
            assert!(
                transfer(&argv, b"image", Duration::from_millis(30), OriginShell::Posix).is_err(),
                "{script}"
            );
        }
        assert!(upload_command("-oProxyCommand=bad", "png", OriginShell::Posix).is_err());
        assert!(upload_command("host", "png;id", OriginShell::Posix).is_err());
    }
}
