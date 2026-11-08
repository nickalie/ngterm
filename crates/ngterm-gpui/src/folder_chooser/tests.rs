use gpui_kit::{Context, IntoElement, Render, TestAppContext, Window};

use super::*;

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div()
    }
}

fn parent(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.add_window(|_, _| Blank).into()
}

fn request(start: Option<PathBuf>) -> FolderRequest {
    FolderRequest {
        title: "Choose a folder",
        accept: "Open Directory",
        start,
    }
}

#[cfg(target_os = "linux")]
pub(super) mod portal {
    use std::cell::RefCell;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    use ashpd::Uri;
    use ashpd::desktop::file_chooser::OpenFileRequest;

    use super::*;

    struct Portal {
        start: Option<PathBuf>,
        response: Result<Option<Vec<Uri>>, ashpd::Error>,
    }

    thread_local! {
        static NEXT: RefCell<Option<Portal>> = const { RefCell::new(None) };
    }

    /// A one-shot transport: an unconfigured call fails here, never falling through to D-Bus.
    pub(crate) async fn send(chooser: OpenFileRequest) -> Result<Option<Vec<Uri>>, ashpd::Error> {
        let portal = NEXT.take().expect("configure the folder chooser's test transport");
        // ashpd exposes the builder's fields only through Debug; inspect the request at the transport boundary.
        let sent = format!("{chooser:?}");
        for field in [
            "identifier: None",
            "title: \"Choose a folder\"",
            "accept_label: Some(\"Open Directory\")",
            "modal: Some(true)",
            "multiple: Some(false)",
            "directory: Some(true)",
        ] {
            assert!(sent.contains(field), "{sent} lacks {field}");
        }
        let start = portal
            .start
            .map(|path| CString::new(path.as_os_str().as_bytes()).unwrap());
        let field = start.map_or_else(
            || "current_folder: None".to_owned(),
            |path| format!("current_folder: Some(FilePath({path:?}))"),
        );
        assert!(sent.contains(&field), "{sent} lacks {field}");
        portal.response
    }

    fn reply(start: Option<PathBuf>, uris: Option<&[&str]>) {
        let response = Ok(uris.map(|uris| uris.iter().map(|uri| Uri::parse(uri).unwrap()).collect()));
        NEXT.set(Some(Portal { start, response }));
    }

    #[gpui_kit::test]
    async fn choose_sends_the_request_and_returns_the_first_folder(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        let selected = folder.path().join("a folder 日本語");
        let uri = url::Url::from_file_path(&selected).unwrap();
        reply(Some(folder.path().to_owned()), Some(&[uri.as_str(), "file:///ignored"]));
        let parent = parent(cx);
        let task = cx.update(|cx| choose(parent, request(Some(folder.path().to_owned())), cx));
        assert_eq!(task.await, Some(selected));
        assert!(NEXT.take().is_none(), "the transport was called");
    }

    #[gpui_kit::test]
    async fn cancel_uses_the_desktops_start_for_absent_missing_or_file_paths(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("file");
        std::fs::write(&file, "not a directory").unwrap();
        let parent = parent(cx);
        for start in [None, Some(folder.path().join("gone")), Some(file)] {
            reply(None, None);
            let task = cx.update(|cx| choose(parent, request(start), cx));
            assert_eq!(task.await, None);
            assert!(NEXT.take().is_none());
        }
    }

    #[gpui_kit::test]
    async fn a_portal_error_returns_no_folder(cx: &mut TestAppContext) {
        NEXT.set(Some(Portal {
            start: None,
            response: Err(ashpd::Error::NoResponse),
        }));
        let parent = parent(cx);
        let task = cx.update(|cx| choose(parent, request(None), cx));
        assert_eq!(task.await, None);
        assert!(NEXT.take().is_none());
    }

    #[gpui_kit::test]
    async fn closing_the_parent_before_opening_still_allows_cancel(cx: &mut TestAppContext) {
        let parent = parent(cx);
        cx.update(|cx| parent.update(cx, |_, window, _| window.remove_window()).unwrap());
        reply(None, None);
        let task = cx.update(|cx| choose(parent, request(None), cx));
        assert_eq!(task.await, None);
        assert!(NEXT.take().is_none());
    }

    #[gpui_kit::test]
    async fn an_empty_remote_or_malformed_selection_returns_no_folder(cx: &mut TestAppContext) {
        let parent = parent(cx);
        for uris in [&[][..], &["https://example.com/folder"][..], &["http://["][..]] {
            reply(None, Some(uris));
            let task = cx.update(|cx| choose(parent, request(None), cx));
            assert_eq!(task.await, None);
            assert!(NEXT.take().is_none());
        }
    }
}

#[gpui_kit::test]
async fn the_platform_prompt_accepts_one_directory(cx: &mut TestAppContext) {
    let parent = parent(cx);
    let task = cx.update(|cx| fallback::choose(parent, request(Some("unused start".into())), cx));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files);
        assert!(options.directories);
        assert!(!options.multiple);
        assert_eq!(options.prompt.as_deref(), Some("Open Directory"));
        Some(vec![PathBuf::from("chosen folder")])
    });
    assert_eq!(task.await, Some(PathBuf::from("chosen folder")));
    assert!(!cx.did_prompt_for_paths());
}

#[gpui_kit::test]
async fn the_platform_prompt_returns_no_folder_for_cancel_or_empty_selection(cx: &mut TestAppContext) {
    let parent = parent(cx);
    for response in [None, Some(vec![])] {
        let task = cx.update(|cx| fallback::choose(parent, request(None), cx));
        cx.simulate_path_prompt_response(|_| response);
        assert_eq!(task.await, None);
    }
}

pub(super) mod macos {
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, c_char, c_void};

    use super::*;

    struct Panel {
        request: FolderRequest,
        response: Option<CString>,
    }

    thread_local! {
        static NEXT: RefCell<Option<Panel>> = const { RefCell::new(None) };
    }

    /// The native call's test double copies the strings and completes while its path is still alive.
    pub(crate) unsafe fn ngterm_choose_folder(
        title: *const c_char,
        accept: *const c_char,
        directory: *const c_char,
        context: *mut c_void,
        reply: extern "C" fn(*mut c_void, *const c_char),
    ) {
        let panel = NEXT.take().expect("configure the folder chooser's test panel");
        // SAFETY: choose passes live C strings for the duration of this call.
        let title = unsafe { CStr::from_ptr(title) };
        // SAFETY: the accept label has the same lifetime as the title.
        let accept = unsafe { CStr::from_ptr(accept) };
        assert_eq!(title.to_bytes(), panel.request.title.as_bytes());
        assert_eq!(accept.to_bytes(), panel.request.accept.as_bytes());
        if let Some(start) = panel.request.start {
            // SAFETY: a representable starting folder is a live C string too.
            let directory = unsafe { CStr::from_ptr(directory) };
            assert_eq!(directory.to_bytes(), start.to_string_lossy().as_bytes());
        } else {
            assert!(directory.is_null());
        }
        reply(
            context,
            panel.response.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()),
        );
    }

    #[gpui_kit::test]
    async fn the_native_panel_receives_the_request_and_returns_its_path(cx: &mut TestAppContext) {
        let parent = parent(cx);
        let request = request(Some("starting folder 日本語".into()));
        NEXT.set(Some(Panel {
            request: request.clone(),
            response: Some(CString::new("chosen folder 日本語").unwrap()),
        }));
        let task = cx.update(|cx| super::super::macos::choose(parent, request, cx));
        assert_eq!(task.await, Some(PathBuf::from("chosen folder 日本語")));
        assert!(NEXT.take().is_none());
    }

    #[gpui_kit::test]
    async fn the_native_panel_returns_no_folder_for_cancel(cx: &mut TestAppContext) {
        let parent = parent(cx);
        NEXT.set(Some(Panel {
            request: request(None),
            response: None,
        }));
        let task = cx.update(|cx| super::super::macos::choose(parent, request(None), cx));
        assert_eq!(task.await, None);
        assert!(NEXT.take().is_none());
    }

    #[gpui_kit::test]
    async fn the_native_panel_handles_unrepresentable_strings(cx: &mut TestAppContext) {
        let parent = parent(cx);
        NEXT.set(Some(Panel {
            request: FolderRequest {
                title: "",
                accept: "",
                start: None,
            },
            response: Some(CString::new(b"folder\xff".to_vec()).unwrap()),
        }));
        let request = FolderRequest {
            title: "title\0suffix",
            accept: "accept\0suffix",
            start: Some("start\0suffix".into()),
        };
        let task = cx.update(|cx| super::super::macos::choose(parent, request, cx));
        assert_eq!(task.await, Some(PathBuf::from("folder\u{fffd}")));
        assert!(NEXT.take().is_none());
    }
}
