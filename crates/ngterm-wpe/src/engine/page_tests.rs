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
        session: crate::SessionId(1),
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
    let page = Page::new(ViewId(5), &spec(), &display, &session, &events).unwrap();
    native_input(&page);
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
        Event::CloseRequested { view: ViewId(5) }
    ));
    assert!(page.web_view.emit_by_name::<bool>("enter-fullscreen", &[]));
    page.terminated(Termination::Crashed);
    assert!(matches!(
        received.try_recv().unwrap(),
        Event::WebProcessTerminated {
            view: ViewId(5),
            reason: Termination::Crashed
        }
    ));
    page.close();
    page.terminated(Termination::TerminatedByApi);
    page.web_view.emit_by_name::<()>("close", &[]);
    assert!(received.is_empty());
    page.end_web_process();
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
}
