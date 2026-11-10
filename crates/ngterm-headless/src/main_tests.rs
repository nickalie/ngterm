use std::ffi::OsString;

use clap::Parser as _;

use super::{Cli, Options, detached_arguments, parse_grid, relaunch, sets_up_agents};

#[test]
fn a_grid_is_columns_by_rows() {
    assert_eq!(parse_grid("120x40"), Ok((120, 40)));
    assert_eq!(parse_grid("80X24"), Ok((80, 24)));
    assert!(parse_grid("120").is_err());
    assert!(parse_grid("1x24").is_err());
    assert!(parse_grid("axb").is_err());
}

#[test]
fn hooks_install_options_are_explicit_and_validate_the_opencode_version() {
    let cli = Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks"]).unwrap();
    assert!(cli.install_agent_hooks);
    assert!(cli.opencode_version.is_none());
    for version in ["v1", "v2"] {
        let cli = Cli::try_parse_from([
            "ngterm-headless",
            "--install-agent-hooks",
            "--opencode-version",
            version,
        ])
        .unwrap();
        assert_eq!(cli.opencode_version.unwrap().raw(), version);
    }
    assert!(Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks", "--opencode-version", "v3"]).is_err());
    assert!(Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks", "--skip-agent-setup"]).is_err());
}

#[test]
fn a_staged_start_sets_up_the_agents_unless_skipped() {
    let parse = |arguments: &[&str]| {
        let cli = Cli::try_parse_from([&["ngterm-headless"], arguments].concat()).unwrap();
        Options {
            grid: cli.grid,
            skip_agent_setup: cli.skip_agent_setup,
            opencode_version: cli.opencode_version,
        }
    };
    let start = parse(&[]);
    assert!(sets_up_agents(&start, true));
    assert!(
        !sets_up_agents(&start, false),
        "a cargo-target build leaves the configs alone"
    );
    let pinned = parse(&["--opencode-version", "v2"]);
    assert!(sets_up_agents(&pinned, true));
    assert_eq!(pinned.opencode_version.unwrap().raw(), "v2");
    assert!(!sets_up_agents(&parse(&["--skip-agent-setup"]), true));
    assert!(
        sets_up_agents(&parse(&["--relaunched"]), true),
        "a relaunch may be an upgraded binary"
    );
    assert!(Cli::try_parse_from(["ngterm-headless", "--skip-agent-setup", "--opencode-version", "v1"]).is_err());
}

#[test]
fn a_detached_servers_command_line_is_the_same_without_the_flag() {
    let arguments = ["--detach", "--grid", "120x40", "--skip-agent-setup"].map(OsString::from);
    assert_eq!(
        detached_arguments(arguments.into_iter()),
        ["--grid", "120x40", "--skip-agent-setup"].map(OsString::from)
    );
    assert!(Cli::try_parse_from(["ngterm-headless", "--detach", "--install-agent-hooks"]).is_err());
}

#[test]
fn a_relaunch_starts_with_the_same_grid_and_waits_for_the_lock() {
    let mut arguments = vec!["ngterm-headless".into()];
    arguments.extend(relaunch::arguments(&Options {
        grid: (120, 40),
        skip_agent_setup: true,
        ..Options::default()
    }));
    let cli = Cli::try_parse_from(arguments).unwrap();
    assert_eq!(
        (cli.grid, cli.relaunched, cli.skip_agent_setup),
        ((120, 40), true, true)
    );
    let cli = Cli::try_parse_from(["ngterm-headless"]).unwrap();
    assert_eq!((cli.grid, cli.relaunched), ((80, 24), false));
}

#[test]
fn grid_errors_preserve_the_offending_count() {
    for (input, error) in [
        ("80", "expected COLUMNSxROWS, as 120x40"),
        ("1x24", "\"1\" is not a count of 2 or more"),
        ("80x0", "\"0\" is not a count of 2 or more"),
        ("65536x24", "\"65536\" is not a count of 2 or more"),
        ("80xabc", "\"abc\" is not a count of 2 or more"),
    ] {
        assert_eq!(parse_grid(input).unwrap_err(), error);
    }
    assert_eq!(parse_grid(" 120 X 40 "), Ok((120, 40)));
}

#[test]
fn claim_refuses_a_second_owner_and_a_relaunch_takes_a_released_lock() {
    let dir = tempfile::tempdir().unwrap();
    let owner = super::claim(dir.path(), false).unwrap();
    assert_eq!(
        super::claim(dir.path(), false).unwrap_err(),
        format!(
            "another instance is already running on {}; control it with ngtermctl",
            dir.path().display()
        )
    );
    let release = std::thread::spawn(move || drop(owner));
    let next = super::claim(dir.path(), true).unwrap();
    release.join().unwrap();
    drop(next);
    assert!(super::claim(dir.path(), false).is_ok());
}

#[cfg(windows)]
#[test]
fn only_a_real_install_with_a_cli_touches_the_users_path() {
    use std::collections::HashMap;

    let vars = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    };
    assert!(super::puts_the_cli_on_path(&vars(&[("PATH", "x")]), true));
    assert!(
        !super::puts_the_cli_on_path(&vars(&[("PATH", "x")]), false),
        "no CLI to find"
    );
    for name in ["NGTERM_STATE_DIR", "ngterm_state_dir"] {
        assert!(
            !super::puts_the_cli_on_path(&vars(&[(name, r"C:\private")]), true),
            "{name} is a private state directory"
        );
    }
    assert!(
        super::puts_the_cli_on_path(&vars(&[("NGTERM_STATE_DIR", "")]), true),
        "an empty variable is unset"
    );
}
