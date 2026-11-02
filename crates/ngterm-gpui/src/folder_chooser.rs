//! The desktop's folder chooser (upstream's `NSOpenPanel` for directories, the GTK build's `GtkFileDialog`): the
//! sidebar's Open Directory… and the Settings popup's Choose… buttons. On Linux it is the file chooser portal, as
//! GTK's `FileDialog` and GPUI's own prompt use it, modal over the asking window (an X11 id or an `xdg-foreign`
//! export), titled and starting in a folder, which GPUI's prompt cannot do; elsewhere GPUI's platform prompt.

use std::path::PathBuf;

use gpui_kit::{AnyWindowHandle, App, Task};

/// What the chooser asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRequest {
    /// The chooser's title (upstream's panel message).
    pub title: &'static str,
    /// Its accept button.
    pub accept: &'static str,
    /// The folder it opens in; the desktop's choice when `None` or gone.
    pub start: Option<PathBuf>,
}

/// Opens the chooser modal over `parent`; the folder picked, `None` when cancelled or without a chooser (logged).
pub fn choose(parent: AnyWindowHandle, request: FolderRequest, cx: &mut App) -> Task<Option<PathBuf>> {
    platform::choose(parent, request, cx)
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    use gpui_kit::{AnyWindowHandle, App, AppContext as _, Task, Window};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

    use super::FolderRequest;

    pub fn choose(parent: AnyWindowHandle, request: FolderRequest, cx: &mut App) -> Task<Option<PathBuf>> {
        cx.spawn(async move |cx| {
            // read from the live window as the chooser opens, so the handles never outlive it
            let handles = cx
                .update_window(parent, |_, window, _| raw_handles(window))
                .ok()
                .flatten();
            let parent = match handles {
                Some((window, display)) => ashpd::WindowIdentifier::from_raw_handle(&window, Some(&display)).await,
                None => None,
            };
            pick(request, parent).await
        })
    }

    /// `window`'s raw window and display handles (an X11 window id, or a Wayland surface and display), which the
    /// portal names its parent by; `None` when the platform has none.
    fn raw_handles(window: &Window) -> Option<(RawWindowHandle, RawDisplayHandle)> {
        let raw = HasWindowHandle::window_handle(window).ok()?.as_raw();
        let display = HasDisplayHandle::display_handle(window).ok()?.as_raw();
        Some((raw, display))
    }

    /// The portal's folder chooser, modal over `parent`.
    async fn pick(request: FolderRequest, parent: Option<ashpd::WindowIdentifier>) -> Option<PathBuf> {
        use ashpd::desktop::file_chooser::OpenFileRequest;
        let base = || {
            OpenFileRequest::default()
                .title(request.title)
                .accept_label(request.accept)
                .modal(true)
                .multiple(false)
                .directory(true)
        };
        let start = request.start.clone().filter(|folder| folder.is_dir());
        let chooser = match start {
            Some(folder) => base().current_folder(folder).unwrap_or_else(|_| base()),
            None => base(),
        };
        let chooser = match chooser.identifier(parent).send().await {
            Ok(chooser) => chooser,
            Err(err) => {
                tracing::warn!(%err, "the folder chooser did not open");
                return None;
            }
        };
        let selected = chooser.response().ok()?;
        selected
            .uris()
            .first()
            .and_then(|uri| url::Url::parse(uri.as_str()).ok())
            .and_then(|url| url.to_file_path().ok())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use std::path::PathBuf;

    use gpui_kit::{AnyWindowHandle, App, PathPromptOptions, Task};

    use super::FolderRequest;

    /// GPUI's prompt takes the accept button's label only: no title, no starting folder.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the Linux form's signature, whose portal request takes it"
    )]
    pub fn choose(_: AnyWindowHandle, request: FolderRequest, cx: &mut App) -> Task<Option<PathBuf>> {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(request.accept.into()),
        });
        cx.spawn(async move |_| match picked.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Ok(None)) | Err(_) => None,
            Ok(Err(err)) => {
                tracing::warn!(%err, "the folder chooser did not open");
                None
            }
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{AnyWindowHandle, App, FolderRequest, PathBuf, Task};
    use std::ffi::{CStr, CString, c_char, c_void};
    type Reply = async_channel::Sender<Option<PathBuf>>;
    unsafe extern "C" {
        fn ngterm_choose_folder(
            title: *const c_char,
            accept: *const c_char,
            directory: *const c_char,
            context: *mut c_void,
            reply: extern "C" fn(*mut c_void, *const c_char),
        );
    }
    extern "C" fn chosen(context: *mut c_void, path: *const c_char) {
        // SAFETY: completion is called once, consuming the sender allocated in choose.
        let reply = unsafe { Box::from_raw(context.cast::<Reply>()) };
        let path = if path.is_null() {
            None
        } else {
            // SAFETY: the open panel's UTF-8 path lives for this callback.
            Some(PathBuf::from(
                unsafe { CStr::from_ptr(path) }.to_string_lossy().into_owned(),
            ))
        };
        let _ = reply.try_send(path);
    }
    pub fn choose(_: AnyWindowHandle, request: FolderRequest, cx: &mut App) -> Task<Option<PathBuf>> {
        let title = CString::new(request.title).unwrap_or_default();
        let accept = CString::new(request.accept).unwrap_or_default();
        let directory = request
            .start
            .and_then(|path| CString::new(path.to_string_lossy().as_bytes()).ok());
        let (reply, result) = async_channel::bounded(1);
        // SAFETY: main-thread adapter copies strings and transfers the sender to its asynchronous completion.
        unsafe {
            ngterm_choose_folder(
                title.as_ptr(),
                accept.as_ptr(),
                directory.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()),
                Box::into_raw(Box::new(reply)).cast(),
                chosen,
            );
        };
        cx.spawn(async move |_| result.recv().await.ok().flatten())
    }
}
