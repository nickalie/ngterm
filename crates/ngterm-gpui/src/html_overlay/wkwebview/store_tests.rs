//! Where `WKWebsiteDataStore`'s profiles are named. The data itself lives in `WebKit`'s stores, keyed by those UUIDs.

use uuid::Uuid;

use super::*;

#[test]
fn profiles_are_named_by_kind_and_id() {
    let id = Uuid::nil();
    assert_eq!(saved_profile(id), "ngterm-00000000-0000-0000-0000-000000000000");
    assert_eq!(page_profile(id), "page-00000000-0000-0000-0000-000000000000");
}

#[test]
fn the_folders_sit_under_the_state_directory() {
    let state = Path::new("/state");
    assert_eq!(user_data_folder(state), Path::new("/state/browser/wkwebview"));
    assert_eq!(profile_folder(state, "ngterm-x"), Path::new("/state/ngterm-x"));
}

#[test]
fn no_page_profile_keeps_a_folder_to_sweep() {
    let state = tempfile::tempdir().unwrap();
    let left = state.path().join("page-left");
    std::fs::create_dir(&left).unwrap();
    assert_eq!(page_profile_folders(state.path()), Vec::<PathBuf>::new());
    remove_later(left.clone());
    assert!(left.is_dir(), "WebKit owns its data: nothing on disk is removed here");
}
