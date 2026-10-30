//! The browser stores of `WebView2` pages: one `WebView2` environment per state directory, its user data folder
//! `<state>/browser/webview2`, holding a profile per store (`WebView2`'s multi-profile support). The saved store is
//! the profile named by the state directory's browser profile id (`ngterm_core::html::profile`, as the WPE host keys
//! `<state>/browser/<id>/` by it), so saved logins outlive the page, the session and the app; every other page gets
//! an `InPrivate` profile of its own, kept in memory and gone with the page. `WebView2` still makes a folder for each
//! such profile, which is removed once the page closed, and any left behind (the app ended first) when the next
//! environment is made.

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// The environment's user data folder under the state directory.
#[must_use]
pub fn user_data_folder(state_dir: &Path) -> PathBuf {
    state_dir.join("browser").join("webview2")
}

/// The saved store's profile name, by the browser profile id.
#[must_use]
pub fn saved_profile(id: Uuid) -> String {
    format!("ngterm-{}", id.hyphenated())
}

/// Page `page`'s in-memory profile name.
#[must_use]
pub fn page_profile(page: Uuid) -> String {
    format!("{PAGE_PREFIX}{}", page.hyphenated())
}

const PAGE_PREFIX: &str = "page-";
/// The folder `WebView2` keeps a named profile in, under the user data folder.
const PROFILE_FOLDER_PREFIX: &str = "WV2Profile_";

/// Where `WebView2` keeps the profile `name` of the environment at `user_data`.
#[must_use]
pub fn profile_folder(user_data: &Path, name: &str) -> PathBuf {
    user_data
        .join("EBWebView")
        .join(format!("{PROFILE_FOLDER_PREFIX}{name}"))
}

/// The in-memory profiles' folders under the environment at `user_data` (the pages' that closed or never closed).
#[must_use]
pub fn page_profile_folders(user_data: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(user_data.join("EBWebView")) else {
        return Vec::new();
    };
    let prefix = format!("{PROFILE_FOLDER_PREFIX}{PAGE_PREFIX}");
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect()
}

/// Removes `folder` once `WebView2` let go of it: tried a few times over a few seconds on a thread of its own, since the
/// browser process holds a profile's files a moment past its last page.
pub fn remove_later(folder: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("webview2-profile".to_owned())
        .spawn(move || {
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if !folder.exists() || std::fs::remove_dir_all(&folder).is_ok() {
                    return;
                }
            }
            tracing::debug!(folder = %folder.display(), "a closed page's profile folder stays until the next start");
        });
    if let Err(err) = spawned {
        tracing::debug!(%err, "a closed page's profile folder stays until the next start");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `WebView2`'s profile names: ASCII letters, digits and `-`, at most 64 characters.
    fn valid_profile_name(name: &str) -> bool {
        name.len() <= 64 && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    }

    #[test]
    fn the_stores_are_profiles_of_one_environment_under_the_state_directory() {
        let state = Path::new(r"C:\Users\u\AppData\Local\ngterm");
        let data = user_data_folder(state);
        assert_eq!(data, state.join("browser").join("webview2"));
        let saved = saved_profile(Uuid::nil());
        let page = page_profile(Uuid::new_v4());
        assert!(valid_profile_name(&saved) && valid_profile_name(&page));
        assert_ne!(
            saved_profile(Uuid::new_v4()),
            saved,
            "the profile id keys the saved store"
        );
        assert_eq!(
            profile_folder(&data, &page),
            data.join("EBWebView").join(format!("WV2Profile_{page}"))
        );
    }

    #[test]
    fn only_the_pages_profile_folders_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let page = profile_folder(dir.path(), &page_profile(Uuid::new_v4()));
        let saved = profile_folder(dir.path(), &saved_profile(Uuid::new_v4()));
        for folder in [&page, &saved] {
            std::fs::create_dir_all(folder).unwrap();
        }
        assert_eq!(page_profile_folders(dir.path()), vec![page]);
        assert!(page_profile_folders(&dir.path().join("missing")).is_empty());
    }
}
