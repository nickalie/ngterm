use super::*;

#[test]
fn navigation_kinds_match_the_consumers_policy_questions() {
    for (raw, expected) in [
        (webkit::WEBKIT_NAVIGATION_TYPE_LINK_CLICKED, NavigationType::LinkClicked),
        (
            webkit::WEBKIT_NAVIGATION_TYPE_FORM_SUBMITTED,
            NavigationType::FormSubmitted,
        ),
        (webkit::WEBKIT_NAVIGATION_TYPE_BACK_FORWARD, NavigationType::BackForward),
        (webkit::WEBKIT_NAVIGATION_TYPE_RELOAD, NavigationType::Reload),
        (
            webkit::WEBKIT_NAVIGATION_TYPE_FORM_RESUBMITTED,
            NavigationType::FormResubmitted,
        ),
        (webkit::WEBKIT_NAVIGATION_TYPE_OTHER, NavigationType::Other),
        (i32::MAX, NavigationType::Other),
    ] {
        assert_eq!(navigation_type_of(raw), expected);
    }
}

#[test]
fn load_failures_keep_the_message_and_classify_only_the_known_domain_and_code() {
    // SAFETY: quark lookups have no preconditions.
    let network = unsafe { webkit::webkit_network_error_quark() };
    // SAFETY: as above.
    let policy = unsafe { webkit::webkit_policy_error_quark() };
    for (domain, code, expected) in [
        (
            network,
            webkit::WEBKIT_NETWORK_ERROR_CANCELLED,
            LoadErrorKind::Cancelled,
        ),
        (
            policy,
            webkit::WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE,
            LoadErrorKind::Interrupted,
        ),
        (
            policy,
            webkit::WEBKIT_POLICY_ERROR_CANNOT_SHOW_MIME_TYPE,
            LoadErrorKind::Interrupted,
        ),
        (network, webkit::WEBKIT_NETWORK_ERROR_TRANSPORT, LoadErrorKind::Failed),
        (policy, webkit::WEBKIT_NETWORK_ERROR_CANCELLED, LoadErrorKind::Failed),
        (
            network,
            webkit::WEBKIT_POLICY_ERROR_CANNOT_SHOW_MIME_TYPE,
            LoadErrorKind::Failed,
        ),
    ] {
        // SAFETY: the domain is a registered quark.
        let domain = unsafe { glib::translate::from_glib(domain) };
        let error = load_error(&glib::Error::with_domain(domain, code, "load refused"));
        assert_eq!(error.kind, expected);
        assert_eq!(error.message, "load refused");
    }
}

fn spec() -> ViewSpec {
    ViewSpec {
        session: crate::SessionId::from_raw(1),
        settings: crate::Settings::overlay(true),
        scripts: Vec::new(),
        message_handlers: Vec::new(),
        zoom: 1.0,
        background: None,
        geometry: Geometry {
            width: 20,
            height: 20,
            scale: 1.0,
        },
        block_network: false,
    }
}

#[test]
fn native_page_refusals_and_closed_pages_report_only_the_allowed_events() {
    crate::tests::isolated(
        "engine::page::tests::native_page_refusals_and_closed_pages_report_only_the_allowed_events",
        || {
            let context = glib::MainContext::new();
            context.with_thread_default(native_page).unwrap();
        },
    );
}

fn native_page() {
    let display = Display::connect(None).unwrap();
    let session = NetworkSession::ephemeral();
    let (events, received) = async_channel::unbounded();
    let appearance = ViewSpec {
        background: Some(Rgba::TRANSPARENT),
        ..spec()
    };
    let page = Page::new(ViewId::from_raw(5), &appearance, &display, &session, &events).unwrap();
    native_appearance(&page);
    native_input(&page);
    native_permission(&page);
    page.apply(ViewCommand::Stop);
    page.apply(ViewCommand::GoForward);
    page.apply(ViewCommand::Scripts(Vec::new()));
    page.apply(ViewCommand::Background(Some(Rgba::TRANSPARENT)));
    page.apply(ViewCommand::Focused(false));
    page.apply(ViewCommand::ImePreedit {
        text: "no focused field".to_owned(),
        cursor: 0,
    });
    page.web_view.emit_by_name::<()>("close", &[]);
    assert!(matches!(
        received.try_recv().unwrap(),
        Event::CloseRequested {
            view
        } if view == ViewId::from_raw(5)
    ));
    assert!(page.web_view.emit_by_name::<bool>("enter-fullscreen", &[]));
    page.terminated(Termination::Crashed);
    assert!(matches!(
        received.try_recv().unwrap(),
        Event::WebProcessTerminated {
            view,
            reason: Termination::Crashed
        } if view == ViewId::from_raw(5)
    ));
    page.close();
    page.terminated(Termination::TerminatedByApi);
    page.web_view.emit_by_name::<()>("close", &[]);
    assert!(received.is_empty());
    page.end_web_process();
}

fn native_appearance(page: &Rc<Page>) {
    let background = || {
        let mut color = webkit::WebKitColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        };
        // SAFETY: a live web view and a writable color out-pointer.
        unsafe { webkit::webkit_web_view_get_background_color(page.web_view.raw(), &raw mut color) };
        [color.red, color.green, color.blue, color.alpha]
    };
    assert!(background().into_iter().all(|channel| channel.abs() < f64::EPSILON));
    page.apply(ViewCommand::Background(None));
    assert!(
        background()
            .into_iter()
            .all(|channel| (channel - 1.0).abs() < f64::EPSILON)
    );
    page.apply(ViewCommand::Zoom(1.75));
    // SAFETY: a live web view.
    let zoom = unsafe { webkit::webkit_web_view_get_zoom_level(page.web_view.raw()) };
    assert!((zoom - 1.75).abs() < f64::EPSILON);
    page.apply(ViewCommand::Visible(false));
    // SAFETY: a live WPE view.
    let visible = unsafe { platform::wpe_view_get_visible(page.view.as_wpe()) };
    assert_eq!(visible, glib::ffi::GFALSE);
    page.apply(ViewCommand::Visible(true));
    // SAFETY: as above.
    let visible = unsafe { platform::wpe_view_get_visible(page.view.as_wpe()) };
    assert_ne!(visible, glib::ffi::GFALSE);
}

fn wait_event<T>(
    host: &crate::Host,
    events: &async_channel::Receiver<Event>,
    mut pick: impl FnMut(Event) -> Option<T>,
) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match events.try_recv() {
            Ok(Event::Policy(request)) => host.decide_policy(request.token, true),
            Ok(Event::Frame(frame)) => host.frame_presented(frame.view, frame.buffer),
            Ok(event) => {
                if let Some(found) = pick(event) {
                    return found;
                }
            }
            Err(async_channel::TryRecvError::Empty) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(error) => panic!("engine events ended: {error}"),
        }
        assert!(std::time::Instant::now() < deadline, "page event timed out");
    }
}

#[test]
fn web_dialogs_are_dismissed_without_blocking_the_page() {
    crate::tests::isolated(
        "engine::page::tests::web_dialogs_are_dismissed_without_blocking_the_page",
        || {
            let (host, events) = crate::Host::start(crate::HostConfig {
                frames: crate::FrameMode::Shm,
                schemes: Vec::new(),
            })
            .unwrap();
            let session = host.create_session(crate::SessionSpec::Ephemeral);
            let view = host.create_view(ViewSpec { session, ..spec() });
            host.load(view, Load::Html { html: "<script>alert('notice'); document.title = JSON.stringify([confirm('continue?'), prompt('name?')]);</script>".to_owned(), base_uri: None });
            let title = wait_event(&host, &events, |event| match event {
                Event::Page { view: id, page } if id == view => page.title,
                _ => None,
            });
            assert_eq!(title, "[false,null]");
            host.shutdown();
        },
    );
}

fn native_input(page: &Page) {
    let view = page.view.as_wpe();
    let event = key_event(
        view,
        42,
        Key {
            pressed: false,
            keyval: 0x00ff_ffff,
            keycode: None,
            modifiers: crate::Modifiers::ALT,
        },
    );
    assert!(!event.is_null());
    // SAFETY: a keyboard event made above, live until the unref below.
    assert_eq!(unsafe { platform::wpe_event_keyboard_get_keycode(event) }, 0);
    // SAFETY: as above.
    assert_eq!(unsafe { platform::wpe_event_keyboard_get_keyval(event) }, 0x00ff_ffff);
    // SAFETY: drops the only reference to the event made above.
    unsafe { platform::wpe_event_unref(event) };
    page.set_geometry(Geometry {
        width: 0,
        height: u32::MAX,
        scale: f64::NAN,
    });
    let toplevel = page.view.toplevel().unwrap();
    let mut width = 0;
    let mut height = 0;
    // SAFETY: a live toplevel and writable size out-pointers.
    unsafe { platform::wpe_toplevel_get_size(toplevel.as_wpe(), &raw mut width, &raw mut height) };
    assert_eq!((width, height), (1, i32::MAX));
    // SAFETY: a live toplevel.
    assert!((unsafe { platform::wpe_toplevel_get_scale(toplevel.as_wpe()) } - 1.0).abs() < f64::EPSILON);
    // SAFETY: a live view; null detaches its toplevel, which we retain above.
    unsafe { platform::wpe_view_set_toplevel(view, std::ptr::null_mut()) };
    assert!(page.view.toplevel().is_none());
    page.set_geometry(Geometry {
        width: 10,
        height: 10,
        scale: 2.0,
    });
    // SAFETY: the retained toplevel and writable size out-pointers.
    unsafe { platform::wpe_toplevel_get_size(toplevel.as_wpe(), &raw mut width, &raw mut height) };
    assert_eq!((width, height), (1, i32::MAX), "a detached toplevel is not resized");
    // SAFETY: the view and its retained toplevel are live.
    unsafe { platform::wpe_view_set_toplevel(view, toplevel.as_wpe()) };
    assert_eq!(page.view.toplevel(), Some(toplevel));
}

fn enum_value(type_: glib::types::Type, value: i32) -> glib::Value {
    glib::EnumClass::with_type(type_).unwrap().to_value(value).unwrap()
}

#[test]
fn native_load_failures_and_termination_signals_keep_their_payloads() {
    crate::tests::isolated(
        "engine::page::tests::native_load_failures_and_termination_signals_keep_their_payloads",
        || {
            let context = glib::MainContext::new();
            context.with_thread_default(load_signals).unwrap();
        },
    );
}

fn load_signals() {
    let display = Display::connect(None).unwrap();
    let session = NetworkSession::ephemeral();
    let (events, received) = async_channel::unbounded();
    let page = Page::new(ViewId::from_raw(8), &spec(), &display, &session, &events).unwrap();
    // SAFETY: registers the native enum type.
    let load_type = unsafe { webkit::webkit_load_event_get_type() };
    // SAFETY: the type was registered above.
    let load_type = unsafe { glib::translate::from_glib(load_type) };
    // SAFETY: registers the native enum type.
    let termination_type = unsafe { webkit::webkit_web_process_termination_reason_get_type() };
    // SAFETY: the type was registered above.
    let termination_type = unsafe { glib::translate::from_glib(termination_type) };
    // SAFETY: a plain error-domain quark lookup.
    let domain = unsafe { webkit::webkit_network_error_quark() };
    // SAFETY: the quark was registered above.
    let domain = unsafe { glib::translate::from_glib(domain) };
    let error = glib::Error::with_domain(domain, webkit::WEBKIT_NETWORK_ERROR_TRANSPORT, "connection lost");
    let load = enum_value(load_type, webkit::WEBKIT_LOAD_STARTED);
    assert!(
        page.web_view
            .emit_by_name::<bool>("load-failed", &[&load, &None::<String>, &error],)
    );
    let Event::LoadFailed {
        view,
        uri,
        error: failure,
    } = received.try_recv().unwrap()
    else {
        panic!("a load failure");
    };
    assert_eq!(view, ViewId::from_raw(8));
    assert_eq!(uri, "");
    assert_eq!(failure.kind, LoadErrorKind::Failed);
    assert_eq!(failure.message, "connection lost");
    let reason = enum_value(termination_type, webkit::WEBKIT_WEB_PROCESS_EXCEEDED_MEMORY_LIMIT);
    page.web_view.emit_by_name::<()>("web-process-terminated", &[&reason]);
    assert!(matches!(
        received.try_recv().unwrap(),
        Event::WebProcessTerminated {
            view,
            reason: Termination::ExceededMemoryLimit
        } if view == ViewId::from_raw(8)
    ));
    // dropped unclosed, so only the handlers' weak hold on the page keeps these from reporting it
    let web_view = page.web_view.clone();
    drop(page);
    web_view.emit_by_name::<()>("load-changed", &[&load]);
    assert!(web_view.emit_by_name::<bool>("load-failed", &[&load, &None::<String>, &error]));
    web_view.emit_by_name::<()>("web-process-terminated", &[&reason]);
    web_view.emit_by_name::<()>("close", &[]);
    web_view.notify("title");
    assert!(
        received.is_empty(),
        "late native callbacks cannot report a dropped page"
    );
}

glib::wrapper! {
    /// `WebKitPermissionRequest`.
    struct PermissionRequest(Interface<webkit::WebKitPermissionRequest, webkit::WebKitPermissionRequestInterface>);
    match fn { type_ => || webkit::webkit_permission_request_get_type(), }
}

// SAFETY: the interface struct is the header's (the -sys crate's layout tests check it); only its methods are set.
unsafe impl<T: glib::subclass::types::ObjectSubclass> glib::subclass::types::IsImplementable<T> for PermissionRequest {
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.allow = Some(allow_request);
        iface.deny = Some(deny_request);
    }
}

extern "C" fn allow_request(request: *mut webkit::WebKitPermissionRequest) {
    decide_request(request, "allow");
}

extern "C" fn deny_request(request: *mut webkit::WebKitPermissionRequest) {
    decide_request(request, "deny");
}

fn decide_request(request: *mut webkit::WebKitPermissionRequest, decision: &'static str) {
    use glib::subclass::prelude::ObjectSubclassIsExt as _;
    // SAFETY: `WebKit` decides a live request, here always a `FakeRequest`; borrowing takes no reference.
    let object: glib::translate::Borrowed<glib::Object> =
        unsafe { glib::translate::from_glib_borrow(request.cast::<glib::gobject_ffi::GObject>()) };
    let request = object.downcast_ref::<FakeRequest>().unwrap();
    request.imp().decisions.borrow_mut().push(decision);
}

mod fake {
    use glib::subclass::prelude::*;

    /// A permission request that records how it was decided.
    #[derive(Default)]
    pub(super) struct FakeRequest {
        pub(super) decisions: std::cell::RefCell<Vec<&'static str>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FakeRequest {
        const NAME: &'static str = "NgtermWpeTestPermissionRequest";
        type Type = super::FakeRequest;
        type Interfaces = (super::PermissionRequest,);
    }

    impl ObjectImpl for FakeRequest {}
}

glib::wrapper! {
    struct FakeRequest(ObjectSubclass<fake::FakeRequest>) @implements PermissionRequest;
}

/// The page denies every permission request and claims it, so neither a later handler nor `WebKit`'s default (which
/// allows a pointer lock) decides it. A page's real geolocation request cannot show this: `WebKit` denies an undecided
/// one on its own.
fn native_permission(page: &Page) {
    use glib::subclass::prelude::ObjectSubclassIsExt as _;
    let request: FakeRequest = glib::Object::new();
    let later = Rc::new(Cell::new(false));
    let seen = Rc::clone(&later);
    let spy = page.web_view.connect_local("permission-request", false, move |_| {
        seen.set(true);
        handled(false)
    });
    assert!(page.web_view.emit_by_name::<bool>("permission-request", &[&request]));
    assert_eq!(*request.imp().decisions.borrow(), ["deny"]);
    assert!(!later.get(), "a refused request goes no further");
    page.web_view.disconnect(spy);
}
