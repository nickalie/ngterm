//! The catalog tests' check of a build's names against the conformance catalog plus a Rust-only list
//! ([`crate::command::RUST_ONLY_COMMANDS`], [`crate::command::RUST_ONLY_CLI`]).
//!
//! Test tooling only: built for this crate's tests and, through the `catalog-check` feature, for another crate's
//! (`ngterm-ctl`'s command tree test enables it as a dev-dependency), never into a release binary. The catalog test
//! (`tests/catalog.rs`) takes this file as a module of its own.

use std::collections::BTreeSet;

/// Checks a build's names against the catalog's names plus a Rust-only list: the build has exactly the catalog's
/// names and the listed ones, and no listed name is in the catalog or listed twice. The error names the first
/// offending name and why.
///
/// # Errors
///
/// The first name in the Rust-only list and the catalog both, or twice in the list, or one the build has that neither
/// lists, or one either lists that the build lacks.
pub fn check_rust_only(ours: &BTreeSet<String>, catalog: &BTreeSet<String>, rust_only: &[&str]) -> Result<(), String> {
    let mut listed = BTreeSet::new();
    for &name in rust_only {
        if catalog.contains(name) {
            return Err(format!("{name}: in the Rust-only list and in the catalog"));
        }
        if !listed.insert(name.to_owned()) {
            return Err(format!("{name}: twice in the Rust-only list"));
        }
    }
    if let Some(name) = ours
        .iter()
        .find(|name| !catalog.contains(*name) && !listed.contains(*name))
    {
        return Err(format!("{name}: neither in the catalog nor in the Rust-only list"));
    }
    if let Some(name) = catalog.iter().chain(&listed).find(|name| !ours.contains(*name)) {
        return Err(format!("{name}: listed but not implemented"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn the_catalog_plus_the_rust_only_list_is_the_protocol() {
        let catalog = names(&["tree", "session.new"]);
        assert_eq!(check_rust_only(&catalog, &catalog, &[]), Ok(()));
        let ours = names(&["tree", "session.new", "agent.get"]);
        assert_eq!(check_rust_only(&ours, &catalog, &["agent.get"]), Ok(()));
    }

    #[test]
    fn a_name_in_both_the_list_and_the_catalog_fails() {
        let catalog = names(&["tree", "session.new"]);
        assert_eq!(
            check_rust_only(&catalog, &catalog, &["tree"]),
            Err("tree: in the Rust-only list and in the catalog".to_owned())
        );
    }

    #[test]
    fn a_rust_only_name_missing_from_the_list_fails() {
        let catalog = names(&["tree"]);
        let ours = names(&["tree", "agent.get", "agent.report"]);
        assert_eq!(
            check_rust_only(&ours, &catalog, &["agent.report"]),
            Err("agent.get: neither in the catalog nor in the Rust-only list".to_owned())
        );
    }

    #[test]
    fn a_listed_name_the_build_lacks_fails() {
        let catalog = names(&["tree", "session.new"]);
        let ours = names(&["tree", "agent.get"]);
        assert_eq!(
            check_rust_only(&ours, &catalog, &["agent.get"]),
            Err("session.new: listed but not implemented".to_owned())
        );
        assert_eq!(
            check_rust_only(&catalog, &catalog, &["agent.get"]),
            Err("agent.get: listed but not implemented".to_owned())
        );
    }

    #[test]
    fn a_name_listed_twice_fails() {
        let catalog = names(&["tree"]);
        let ours = names(&["tree", "agent.get"]);
        assert_eq!(
            check_rust_only(&ours, &catalog, &["agent.get", "agent.get"]),
            Err("agent.get: twice in the Rust-only list".to_owned())
        );
    }
}
