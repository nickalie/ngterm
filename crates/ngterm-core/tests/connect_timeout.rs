//! The process-wide connect timeouts as the app reads them: unit tests keep one table per thread, so only a test
//! binary of its own reaches the table every thread of the app shares.

use std::time::Duration;

use ngterm_core::remote::connect_timeout::after_connect;
use ngterm_core::remote::remotes::RemotesState;
use ngterm_core::remote::tree::ssh_arguments;

fn option(host: &str) -> String {
    let argv = ssh_arguments(host, false);
    argv.iter()
        .find(|word| word.starts_with("ConnectTimeout="))
        .cloned()
        .unwrap_or_default()
}

#[test]
fn every_thread_reads_what_remotes_conf_last_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remotes.conf");
    assert_eq!(option("studio"), "ConnectTimeout=10");
    std::fs::write(&path, "connect-timeout = 20\nconnect-timeout studio = 45\nstudio\n").unwrap();
    let mut state = RemotesState::at(path.clone());
    let elsewhere = std::thread::spawn(|| (option("studio"), option("mini"), after_connect("mini", Duration::ZERO)));
    assert_eq!(
        elsewhere.join().unwrap(),
        (
            "ConnectTimeout=45".to_owned(),
            "ConnectTimeout=20".to_owned(),
            Duration::from_secs(20)
        )
    );
    std::fs::write(&path, "studio\n").unwrap();
    state.refresh(true);
    let elsewhere = std::thread::spawn(|| option("studio"));
    assert_eq!(
        elsewhere.join().unwrap(),
        "ConnectTimeout=10",
        "an edit reaches the next ssh"
    );
}
