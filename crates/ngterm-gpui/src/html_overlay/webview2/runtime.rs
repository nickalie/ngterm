//! The `WebView2` Runtime, checked once at the app's start (`registry::install`): every page needs it, and a machine
//! without it (Windows 10 without Edge's evergreen runtime, a removed one) would fail each page at its first load.
//! Its absence is logged at the start and refuses every page with a reason that says where to get it.

use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
use windows::core::{PCWSTR, PWSTR};

/// Why pages are refused without the runtime.
pub const MISSING: &str = "the Microsoft Edge WebView2 Runtime is not installed; \
     get it from https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// The installed runtime's version, or why there is none.
///
/// # Errors
///
/// [`MISSING`], with the system's reason when it gave one.
pub fn installed_version() -> Result<String, String> {
    let mut version = PWSTR::null();
    // SAFETY: a query with no browser folder (the installed runtime) into an out string the call allocates, which
    // `take_pwstr` frees.
    let result = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &raw mut version) };
    let text = webview2_com::take_pwstr(version);
    version_of(result.map_err(|err| err.to_string()), text)
}

/// The version a query answered (`result`, then its text): an empty one is no runtime.
fn version_of(result: Result<(), String>, text: String) -> Result<String, String> {
    match result {
        Ok(()) if !text.is_empty() => Ok(text),
        Ok(()) => Err(MISSING.to_owned()),
        Err(reason) => Err(format!("{MISSING} ({reason})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_runtime_says_where_to_get_it() {
        assert_eq!(
            version_of(Ok(()), "140.0.3485.54".to_owned()),
            Ok("140.0.3485.54".to_owned())
        );
        assert_eq!(version_of(Ok(()), String::new()), Err(MISSING.to_owned()));
        let failed = version_of(
            Err("The system cannot find the file specified.".to_owned()),
            String::new(),
        );
        assert!(failed.unwrap_err().starts_with(MISSING));
        assert!(MISSING.contains("https://go.microsoft.com/fwlink/p/?LinkId=2124703"));
    }

    /// The runtime the runner or the VM has: CI's Windows images carry Edge's evergreen runtime.
    #[test]
    fn the_installed_runtime_is_found() {
        let version = installed_version().expect("the WebView2 runtime");
        assert!(version.split('.').count() >= 3, "{version}");
    }
}
