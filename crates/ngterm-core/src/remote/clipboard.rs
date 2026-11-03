//! Image clipboard transfers to POSIX remote origins. The image travels on SSH stdin, outside the terminal PTY,
//! and is retained in a private remote temporary directory so a coding agent can read it after the paste.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
    let argv = upload_command(host, extension)?;
    transfer(&argv, bytes, DEADLINE).map_err(|error| error.to_string())
}

fn upload_command(host: &str, extension: &str) -> Result<Vec<String>, String> {
    if !is_host(host) || !extension.chars().all(|ch| ch.is_ascii_alphanumeric()) || extension.is_empty() {
        return Err("Invalid image upload destination or format".to_owned());
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
fn transfer(argv: &[String], bytes: &[u8], deadline: Duration) -> std::io::Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(std::io::Error::other("Clipboard image is empty or exceeds 64 MiB"));
    }
    let mut input = tempfile::tempfile()?;
    input.write_all(bytes)?;
    input.rewind()?;
    let mut output = tempfile::tempfile()?;
    let mut errors = tempfile::tempfile()?;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
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
    let valid = path
        .strip_prefix("/tmp/ngterm-paste.")
        .and_then(|tail| tail.split_once("/clipboard."))
        .is_some_and(|(id, extension)| {
            id.len() == 10
                && id.chars().all(|ch| ch.is_ascii_alphanumeric())
                && !extension.is_empty()
                && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        });
    if !valid {
        return Err(std::io::Error::other("Image upload returned an invalid filename"));
    }
    Ok(path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn upload_delivers_exact_bytes_to_a_private_remote_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let argv = upload_command("example", "png").unwrap();
        let command = vec!["/bin/sh".to_owned(), "-c".to_owned(), argv.last().unwrap().clone()];
        let bytes = b"\x89PNG\r\n\x1a\n\0\xffbinary\x1b[201~";
        let path = transfer(&command, bytes, Duration::from_secs(5)).unwrap();
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
                transfer(&argv, b"image", Duration::from_millis(30)).is_err(),
                "{script}"
            );
        }
        assert!(upload_command("-oProxyCommand=bad", "png").is_err());
        assert!(upload_command("host", "png;id").is_err());
    }
}
