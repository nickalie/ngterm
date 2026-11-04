//! The protocol's wire names equal the conformance catalog's `commands` and `events`.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use ngterm_proto::command::{RUST_ONLY_CLI, RUST_ONLY_COMMANDS, check_rust_only};
use ngterm_proto::{Command, EventKind};
use serde::Deserialize;
use strum::IntoEnumIterator;

#[derive(Deserialize)]
struct Catalog {
    commands: BTreeMap<String, Entry>,
    events: BTreeMap<String, Entry>,
    cli: BTreeMap<String, serde_norway::Value>,
}

#[derive(Deserialize)]
struct Entry {
    status: String,
}

fn load_catalog() -> Result<Catalog, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/catalog.yaml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(serde_norway::from_str(&text)?)
}

/// Names the catalog still lists as part of upstream's protocol.
fn listed(entries: &BTreeMap<String, Entry>) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|(_, entry)| entry.status != "removed-upstream")
        .map(|(name, _)| name.clone())
        .collect()
}

/// The catalog speaks this build's names: its generator renames upstream's `zmx.*` group to `live.*`
/// (`conformance/tools/gen_catalog.py`), so no wire name here names zmx.
#[test]
fn the_catalog_names_no_zmx_and_the_live_group_is_ours() -> Result<(), Box<dyn Error>> {
    let catalog = load_catalog()?;
    assert!(
        catalog.commands.keys().all(|name| !name.contains("zmx")),
        "{:?}",
        catalog.commands.keys()
    );
    let ours: BTreeSet<String> = Command::all().into_iter().map(|c| c.wire_name().to_owned()).collect();
    assert!(ours.iter().any(|name| name.starts_with("live.")), "{ours:?}");
    assert!(ours.iter().all(|name| !name.contains("zmx")), "{ours:?}");
    Ok(())
}

/// The protocol is upstream's commands, as the catalog lists them, plus this build's Rust-only extensions.
#[test]
fn command_wire_names_equal_catalog_commands() -> Result<(), Box<dyn Error>> {
    let catalog = load_catalog()?;
    let ours: BTreeSet<String> = Command::all().into_iter().map(|c| c.wire_name().to_owned()).collect();
    check_rust_only(&ours, &listed(&catalog.commands), RUST_ONLY_COMMANDS)?;
    Ok(())
}

/// The catalog is generated from upstream, so it never names a Rust-only command or subcommand, not even as removed
/// upstream.
#[test]
fn the_catalog_names_no_rust_only_command() -> Result<(), Box<dyn Error>> {
    let catalog = load_catalog()?;
    for name in RUST_ONLY_COMMANDS {
        assert!(!catalog.commands.contains_key(*name), "{name}");
    }
    for path in RUST_ONLY_CLI {
        assert!(!catalog.cli.contains_key(*path), "{path}");
    }
    Ok(())
}

#[test]
fn every_catalog_command_parses_to_its_own_name() -> Result<(), Box<dyn Error>> {
    let catalog = load_catalog()?;
    for name in listed(&catalog.commands) {
        let command: Command = name.parse()?;
        assert_eq!(command.wire_name(), name);
    }
    Ok(())
}

#[test]
fn event_kinds_equal_catalog_events() -> Result<(), Box<dyn Error>> {
    let catalog = load_catalog()?;
    let ours: BTreeSet<String> = EventKind::iter().map(|k| k.wire_name().to_owned()).collect();
    assert_eq!(ours, listed(&catalog.events));
    Ok(())
}
