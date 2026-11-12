#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::{
    ClipboardHtml, ClipboardMimeError, ClipboardRepresentation, ClipboardTarget, HTML_MIME_TYPE, NativeInput, NativeKey,
    clipboard_mime_types, current_native_input, current_platform, is_text_mime, linux_platform, read_clipboard_mimes,
    with_clipboard_html, write_clipboard_representations, x11_scale_factor, x11_window_maximized, x11_window_minimized,
};
