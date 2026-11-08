//! Read the real TIS sources and select only the source already selected by the user.
use super::*;

#[test]
fn real_input_sources_can_be_read_and_reselected_without_switching_layouts() {
    // TIS aborts when called concurrently, so all native layout checks share this test's thread.
    the_platform_probe_lists_and_reselects_the_current_input_source();
    an_unknown_source_is_rejected_without_changing_the_layout();
    missing_properties_have_no_string_and_are_false();
}

fn the_platform_probe_lists_and_reselects_the_current_input_source() {
    let mut backend = super::super::probe("unused on macOS").unwrap();
    let current = backend.current().unwrap();
    assert!(!current.is_empty());
    let layouts = backend.layouts().unwrap();
    assert!(!layouts.is_empty());
    assert!(layouts.iter().any(|layout| layout.id == current));
    let ids: std::collections::HashSet<_> = layouts.iter().map(|layout| &layout.id).collect();
    assert_eq!(ids.len(), layouts.len(), "the picker must not repeat an input source");
    assert!(
        layouts
            .iter()
            .all(|layout| !layout.id.is_empty() && !layout.name.is_empty())
    );
    backend.select(&current).unwrap();
    assert_eq!(backend.current().unwrap(), current);
}

fn an_unknown_source_is_rejected_without_changing_the_layout() {
    let mut backend = InputSources;
    let current = backend.current().unwrap();
    let missing = "org.ngterm.tests.nonexistent-input-source";
    assert!(source_with_id(missing).is_none());
    assert_eq!(
        backend.select(missing).unwrap_err().to_string(),
        format!("no input source {missing}")
    );
    assert_eq!(backend.current().unwrap(), current);
}

fn missing_properties_have_no_string_and_are_false() {
    let current = InputSources.current().unwrap();
    let list = source_with_id(&current).unwrap();
    let source = each(&list).next().unwrap();
    let missing = CFString::from_str("org.ngterm.tests.nonexistent-property");
    assert_eq!(string(source, &missing), None);
    assert!(!flag(source, &missing));
    // SAFETY: a framework constant whose string property the source owns.
    assert_eq!(string(source, unsafe { kTISPropertyInputSourceID }), Some(current));
}
