//! `WKWebsiteDataStore` UUIDs isolate each state directory's saved profile; incognito pages use memory stores.
use std::path::{Path, PathBuf};
use uuid::Uuid;
pub fn user_data_folder(state: &Path) -> PathBuf {
    state.join("browser/wkwebview")
}
pub fn saved_profile(id: Uuid) -> String {
    format!("ngterm-{id}")
}
pub fn page_profile(id: Uuid) -> String {
    format!("page-{id}")
}
pub fn profile_folder(state: &Path, profile: &str) -> PathBuf {
    state.join(profile)
}
pub fn page_profile_folders(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}
pub fn remove_later(_: PathBuf) {}
