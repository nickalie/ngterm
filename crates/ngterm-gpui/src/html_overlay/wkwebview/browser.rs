//! Hand a validated address to the user's default browser.
pub fn open(uri: &str) -> bool {
    std::process::Command::new("/usr/bin/open")
        .arg("--")
        .arg(uri)
        .status()
        .is_ok_and(|status| status.success())
}
