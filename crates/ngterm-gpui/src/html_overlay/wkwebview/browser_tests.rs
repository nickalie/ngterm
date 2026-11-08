//! Browser hand-off arguments and process results, without starting a browser.
use std::os::unix::process::ExitStatusExt as _;

use super::*;

#[test]
fn the_address_is_one_argument_after_the_end_of_options() {
    for uri in [
        "https://example.com/a b?q=日本語&next=$(false)",
        "file:///tmp/a b/page.html",
        "-a",
    ] {
        let output = tempfile::NamedTempFile::new().unwrap();
        assert!(open_with(uri, |command| {
            assert_eq!(command.get_program(), "/usr/bin/open");
            // The substitute records the arguments as a child would receive them; no address is shell code.
            Command::new("/bin/sh")
                .args(["-c", r#"printf '%s\0' "$@" > "$0""#])
                .arg(output.path())
                .args(command.get_args())
                .status()
        }));
        assert_eq!(std::fs::read(output.path()).unwrap(), format!("--\0{uri}\0").as_bytes());
    }
}

#[test]
fn only_a_successful_process_reports_a_successful_hand_off() {
    for (status, expected) in [(0, true), (1, false), (127, false)] {
        assert_eq!(
            open_with("https://example.com", |_| {
                Command::new("/bin/sh").args(["-c", &format!("exit {status}")]).status()
            }),
            expected
        );
    }
    assert!(!open_with("https://example.com", |_| Ok(ExitStatus::from_raw(9))));
    assert!(!open_with("https://example.com", |_| {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }));
    // Command rejects an embedded NUL before launching /usr/bin/open.
    assert!(!open("https://example.com/\0"));
}
