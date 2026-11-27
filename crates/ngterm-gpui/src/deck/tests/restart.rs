//! A `session.restart` of a pane keeps the page as it was: the split's shown or hidden state, the pane the window shows
//! and the focused pane.

use ngterm_core::live::daemon_name;
use ngterm_host::ProcessRecord;

use super::*;

/// What the page of `session` shows: the plan's laid-out panes and its focused one.
fn layout(h: &mut Harness<'_>, session: Uuid) -> (bool, bool, Option<String>) {
    let primary = h.token(session, StatusPane::Left);
    let split = h
        .session(session)
        .pane_slot(StatusPane::Right)
        .get()
        .map(|s| s.pane_token().to_owned());
    let primary_shown = h.shown(session, &primary);
    let split_shown = split.as_deref().is_some_and(|token| h.shown(session, token));
    let focus = h.deck.read_with(h.cx, |deck, _| {
        deck.page(session)
            .and_then(|page| page.focus())
            .map(|pane| pane.token().to_owned())
    });
    (primary_shown, split_shown, focus)
}

/// Runs `session.restart` of `session`'s `pane` to its answer, the frontend replacing the terminal as the app does.
fn restart(h: &mut Harness<'_>, host: &FakeSessionHost, session: Uuid, pane: StatusPane) {
    let token = h.token(session, pane);
    let identity = Uuid::parse_str(&token).unwrap();
    let daemon = daemon_name(identity);
    host.set_record(&daemon, Some(1), Some(100), None);
    host.set_job(
        100,
        vec![ProcessRecord {
            pid: 101,
            started: 7,
            group: 101,
            foreground: 101,
        }],
    );
    let request = Request {
        cmd: Command::Session(SessionCmd::Restart),
        target: Some(uuid_string(session)),
        args: Some(Args {
            pane_id: Some(token.clone()),
            command: Some("claude".to_owned()),
            ..Args::default()
        }),
    };
    let mut dispatched = h.state.core.begin(&request);
    let deadline = Instant::now() + DEADLINE;
    let response = loop {
        assert!(Instant::now() < deadline, "the restart did not settle");
        match dispatched {
            ngterm_core::Dispatched::Done(response) => break response,
            ngterm_core::Dispatched::Pending { resume, .. } => {
                h.state.settle();
                if replace_panes(h) {
                    host.set_record(&daemon, Some(1), Some(200), None);
                }
                h.turn();
                std::thread::sleep(Duration::from_millis(20));
                dispatched = resume(&mut h.state.core);
            }
        }
    };
    assert!(response.ok, "{response:?}");
    h.turn();
}

/// Puts the terminals the core asked to replace in place, as the app does before its windows render; whether any.
fn replace_panes(h: &mut Harness<'_>) -> bool {
    let replacements = h.state.core.take_pane_replacements();
    for replacement in &replacements {
        let (deck, state) = (h.deck.clone(), &mut h.state);
        let replaced = h.cx.update(|window, cx| {
            deck.update(cx, |deck, cx| {
                deck.replace_pane(&mut state.core, replacement, window, cx)
            })
        });
        assert!(replaced);
        h.state.core.pane_replaced(replacement);
    }
    !replacements.is_empty()
}

/// One live session, selected, with its split shown and the main pane started.
fn split_session<'a>(cx: &'a mut TestAppContext, resources: &Path, host: &Arc<FakeSessionHost>) -> (Harness<'a>, Uuid) {
    let library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let session = library.store(window).unwrap().sessions().next().unwrap().id();
    let core = live_core_on(library, resources, Arc::clone(host) as Arc<dyn SessionHost>);
    let mut h = harness(cx, core, window);
    h.until("the main pane to start", |h| h.realized(session, StatusPane::Left));
    h.store().set_split_visibility(session, true, None);
    h.state.settle();
    h.until("the split pane to start", |h| h.realized(session, StatusPane::Right));
    (h, session)
}

#[gpui_kit::test]
fn a_restart_of_the_main_pane_keeps_a_hidden_split_on_the_main_pane(cx: &mut TestAppContext) {
    let resources = live_resources();
    let host = Arc::new(FakeSessionHost::new());
    let (mut h, session) = split_session(cx, resources.path(), &host);
    h.store().set_split_focus(session, false);
    h.store().set_split_visibility(session, false, None);
    h.state.settle();
    h.turn();
    let before = layout(&mut h, session);
    assert_eq!((before.0, before.1), (true, false), "the main pane shows alone");
    restart(&mut h, &host, session, StatusPane::Left);
    let after = layout(&mut h, session);
    assert_eq!(after, before);
}

/// A relaunch restores a hidden split with no terminal yet; when its right pane held the focus, the window shows that
/// pane again, not the primary, and a restart of the primary keeps it so.
#[gpui_kit::test]
fn a_restored_hidden_split_shows_its_focused_pane_through_a_restart(cx: &mut TestAppContext) {
    let resources = live_resources();
    let host = Arc::new(FakeSessionHost::new());
    let mut library = WindowLibrary::seeded("/home/u");
    let window = library.active_window_id().unwrap();
    let store = library.store_mut(window).unwrap();
    let session = store.sessions().next().unwrap().id();
    let found = store.session_mut(session).unwrap();
    (found.split.exists, found.split.focused) = (true, true);
    found.split_pane_identity = Some(Uuid::new_v4());
    let core = live_core_on(library, resources.path(), Arc::clone(&host) as Arc<dyn SessionHost>);
    let mut h = harness(cx, core, window);
    h.until("both panes to start", |h| {
        h.realized(session, StatusPane::Left) && h.realized(session, StatusPane::Right)
    });
    let split = h.token(session, StatusPane::Right);
    let before = layout(&mut h, session);
    assert_eq!(
        before,
        (false, true, Some(split.clone())),
        "the focused split pane shows alone"
    );
    assert!(h.focused(session, &split));
    restart(&mut h, &host, session, StatusPane::Left);
    assert_eq!(layout(&mut h, session), before);
    assert!(h.focused(session, &split));
}
