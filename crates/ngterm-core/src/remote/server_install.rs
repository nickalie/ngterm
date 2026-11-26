//! `ngtermctl server install`'s host-free rules (upstream's `ServerInstall.swift`): which architectures have a server,
//! how the host's `uname -m` reads, the remote commands that unpack ngterm's Linux release archive under
//! `~/.local/lib/ngterm` and run the installer it carries (`scripts/server-host-install.sh`), and the command's exit
//! code from that installer's. The release asset, its checksum and its download are `ngterm-update`'s.

/// The host architectures a Linux release archive is built for, as `uname -m` names them.
pub const ARCHITECTURES: [&str; 2] = ["aarch64", "x86_64"];

/// The largest archive accepted, downloaded or read from `--from`, in bytes.
pub const DOWNLOAD_LIMIT: u64 = 256 << 20;

/// What the probe runs on the host.
pub const PROBE_COMMAND: &str = "uname -m";

/// The host side of the unpack, with the archive on stdin: unpacked beside the live directory and swapped in by rename,
/// so a running server keeps its open binaries. An archive without the installer is refused before the swap. One line
/// with no quote or backslash, because the account's login shell (fish or tcsh, say) parses the quoted command first
/// and tcsh rejects a quoted newline.
pub const UNPACK_SCRIPT: &str = concat!(
    r#"set -e; lib="$HOME/.local/lib/ngterm"; mkdir -p "$HOME/.local/lib"; rm -rf "$lib.new" "$lib.old"; "#,
    r#"mkdir "$lib.new"; tar -xzf - -C "$lib.new"; test -f "$lib.new/ngterm/usr/share/ngterm/server-install.sh"; "#,
    r#"if [ -e "$lib" ]; then mv "$lib" "$lib.old"; fi; mv "$lib.new/ngterm" "$lib"; rm -rf "$lib.old" "$lib.new""#
);

/// Runs the archive's installer from where the unpack put it.
pub const INSTALLER_COMMAND: &str = r#"bash "$HOME/.local/lib/ngterm/usr/share/ngterm/server-install.sh""#;

/// The remote command of the unpack: [`UNPACK_SCRIPT`] through `/bin/sh`, whatever the account's shell.
#[must_use]
pub fn unpack_command() -> String {
    format!("sh -c '{UNPACK_SCRIPT}'")
}

/// The architecture the probe's output names.
///
/// # Errors
/// The trimmed output, when it names no architecture a release is built for.
pub fn architecture(probe_output: &[u8]) -> Result<&'static str, String> {
    let name = String::from_utf8_lossy(probe_output);
    let name = name.trim();
    ARCHITECTURES
        .into_iter()
        .find(|architecture| *architecture == name)
        .ok_or_else(|| name.to_owned())
}

/// The command's exit code from the installer's exit status (`None` when a signal ended it): 0 installed, 3 installed
/// with the agent hooks incomplete, 1 for anything else.
#[must_use]
pub fn exit_code(status: Option<i32>) -> u8 {
    match status {
        Some(0) => 0,
        Some(3) => 3,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supported_architecture_is_read() {
        for output in ["aarch64\n", "x86_64", "  x86_64 \r\n"] {
            assert!(
                architecture(output.as_bytes()).is_ok_and(|name| ARCHITECTURES.contains(&name)),
                "{output:?}"
            );
        }
    }

    #[test]
    fn any_other_probe_output_is_refused_by_its_trimmed_name() {
        for (output, name) in [
            ("riscv64\n", "riscv64"),
            ("", ""),
            ("arm64", "arm64"),
            ("x86_64\nextra", "x86_64\nextra"),
        ] {
            assert_eq!(architecture(output.as_bytes()), Err(name.to_owned()), "{output:?}");
        }
    }

    #[test]
    fn the_installer_outcome_maps_to_the_commands_exit_code() {
        for (status, code) in [(Some(0), 0), (Some(3), 3), (Some(1), 1), (Some(255), 1), (None, 1)] {
            assert_eq!(exit_code(status), code, "{status:?}");
        }
    }

    #[test]
    fn the_unpack_reads_the_same_in_every_shell() {
        assert!(!UNPACK_SCRIPT.contains(['\'', '\n', '\\']));
        assert_eq!(unpack_command(), format!("sh -c '{UNPACK_SCRIPT}'"));
    }

    #[test]
    fn the_installer_is_the_one_the_release_archive_carries() {
        let package = include_str!("../../../../scripts/package-release.sh");
        assert!(package.contains(r#""$root/usr/share/ngterm/server-install.sh""#));
        assert!(INSTALLER_COMMAND.ends_with(r#"/.local/lib/ngterm/usr/share/ngterm/server-install.sh""#));
    }
}
