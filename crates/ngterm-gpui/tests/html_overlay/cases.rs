//! The cases, run in one child process per GPU against one engine (`WebKit` starts once per process).
//!
//! A [`Fixture`] is a headless GPUI app with one window whose root shows a page's element; every turn runs GPUI's
//! tasks (the engine's events arrive through the overlay's pump), renders a frame and reads it back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, HeadlessAppContext, InputEvent as _, IntoElement, NoopTextSystem,
    ParentElement as _, Render, Styled as _, Window, WindowBounds, WindowHandle, WindowOptions, div, point, px, size,
};
use ngterm_core::html::{HtmlOverlay, HtmlPageFlags, HtmlSource};
use ngterm_core::remote::replica_page::PageResource;
use ngterm_gpui::external_texture::{FrameSource, negotiate};
use ngterm_gpui::html_overlay::{self, FrameStats, PageView, ReplicaLink};
use uuid::Uuid;

/// What a case runs with.
pub struct Gpu {
    pub name: String,
}

pub type Case = fn(&mut Fixture, &Gpu) -> Result<(), String>;

pub const CASES: [(&str, Case); 22] = [
    ("the_engine_renders_on_the_renderers_gpu_in_its_mode", mode),
    ("a_page_loads_and_its_frames_reach_the_screen", loads),
    ("a_page_moved_to_another_window_draws_there", moved),
    (
        "an_animation_keeps_its_frame_rate_on_dmabufs_without_falling_back",
        animation,
    ),
    (
        "a_file_replica_loads_its_page_and_an_asset_through_its_channel",
        replica_loads,
    ),
    ("a_replica_reads_nothing_of_another_page", replica_other_page),
    ("a_slow_replica_read_blocks_no_other", replica_slow),
    ("a_replicas_bridge_reply_keeps_unknown_fields", replica_bridge),
    (
        "a_tagged_control_on_a_replica_relays_with_page_script_off",
        replica_tagged,
    ),
    (
        "the_webview2_guard_keeps_scripted_and_shadowed_clicks_off_file_inputs",
        file_chooser_guard,
    ),
    ("a_granted_file_pages_audio_loads_from_beside_it", granted_audio),
    (
        "a_play_of_a_granted_file_pages_audio_waits_for_its_rescue",
        granted_play,
    ),
    (
        "a_source_replaced_during_a_slow_read_loads_and_the_late_failure_stays_silent",
        replica_audio_replaced,
    ),
    (
        "a_rescued_source_given_another_file_plays_the_new_one",
        granted_source_switched,
    ),
    (
        "a_play_of_an_unreadable_file_rejects_without_another_error",
        granted_unreadable_play,
    ),
    (
        "a_play_survives_rescues_that_rerun_the_selection_twice",
        replica_play_reruns,
    ),
    (
        "a_pause_during_a_rescue_aborts_the_play_and_keeps_the_audio_paused",
        replica_pause_during_rescue,
    ),
    (
        "a_pause_during_a_rescue_keeps_autoplay_audio_paused",
        replica_pause_during_rescue_autoplay,
    ),
    (
        "a_pause_before_a_rescue_keeps_autoplay_audio_paused",
        replica_pause_before_rescue_autoplay,
    ),
    (
        "a_later_sources_failure_stays_silent_while_an_earlier_one_is_rescued",
        granted_later_source_silent,
    ),
    (
        "a_later_sources_held_failure_is_told_once_the_earlier_one_is_removed",
        replica_held_failure_told_on_removal,
    ),
    (
        "a_play_after_a_reload_of_a_file_that_appeared_waits_for_a_fresh_rescue",
        granted_unreadable_reloaded,
    ),
];

/// Runs the cases `filter` selects on `gpu`; the child's exit code.
pub fn run(gpu: &str, filter: Option<&str>) -> i32 {
    let gpu = Gpu { name: gpu.to_owned() };
    let mut fixture = match Fixture::new() {
        Ok(fixture) => fixture,
        Err(message) => {
            println!("  the fixture did not start on {}: {message}", gpu.name);
            return 1;
        }
    };
    let mut failed = 0;
    for (name, case) in CASES {
        if filter.is_some_and(|filter| !name.contains(filter)) {
            continue;
        }
        let started = Instant::now();
        match case(&mut fixture, &gpu) {
            Ok(()) => println!("  {name} ... ok ({:.1?})", started.elapsed()),
            Err(message) => {
                println!("  {name} ... FAILED on {}: {message}", gpu.name);
                failed += 1;
            }
        }
    }
    html_overlay::shutdown();
    i32::from(failed > 0)
}

// MARK: - the fixture

const WAIT: Duration = Duration::from_secs(30);
const RED: [u8; 4] = [0xc0, 0x30, 0x30, 0xff];

/// The window's content: the page shown, at the window's size.
struct Stage {
    element: Option<Entity<PageView>>,
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui_kit::black()).children(self.element.clone())
    }
}

pub struct Fixture {
    cx: HeadlessAppContext,
    window: WindowHandle<Stage>,
    scratch: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(NoopTextSystem),
            Arc::new(()),
            gpui_kit::platform::current_headless_renderer,
        );
        // the engine's events wake the pump from its own thread
        cx.allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            html_overlay::init(cx);
        });
        let window = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(320.), px(240.)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Stage { element: None }),
                )
            })
            .map_err(|error| format!("the window did not open: {error:#}"))?;
        let scratch = tempfile::tempdir().map_err(|err| err.to_string())?;
        let mut fixture = Self { cx, window, scratch };
        // the first frame opens GPUI's device, which the engine renders on
        fixture.frame()?;
        Ok(fixture)
    }

    /// Mounts a chromeless page with its scripts on showing `html` (a file page without a grant: its text is loaded) in
    /// a window of its own `width` x `height`, and shows it.
    fn show(&mut self, html: &str, width: f32, height: f32) -> Result<Entity<PageView>, String> {
        let overlay = self.page(html)?;
        self.mount(&overlay, width, height)
    }

    /// A chromeless file page with its scripts on, showing `html`.
    fn page(&self, html: &str) -> Result<HtmlOverlay, String> {
        let path: PathBuf = self.scratch.path().join(format!("page-{}.html", uuid::Uuid::new_v4()));
        std::fs::write(&path, html).map_err(|err| err.to_string())?;
        Ok(HtmlOverlay::new(
            HtmlSource::File {
                path: path.to_string_lossy().into_owned(),
                grant_root: None,
            },
            HtmlPageFlags {
                chromeless: true,
                // the page's own scripts run (`--js`): the animation is one
                javascript: true,
                ..HtmlPageFlags::default()
            },
        ))
    }

    /// A chromeless file page with its scripts on, granted the scratch directory it is written into as `name`, with
    /// `files` beside it; its pages load through the grant's scheme.
    fn granted_page(&self, name: &str, html: &str, files: &[(&str, &[u8])]) -> Result<HtmlOverlay, String> {
        let root = self.scratch.path().join(format!("site-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).map_err(|err| err.to_string())?;
        for (file, body) in files {
            let path = root.join(file);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
            }
            std::fs::write(path, body).map_err(|err| err.to_string())?;
        }
        let path = root.join(name);
        std::fs::write(&path, html).map_err(|err| err.to_string())?;
        Ok(HtmlOverlay::new(
            HtmlSource::File {
                path: path.to_string_lossy().into_owned(),
                grant_root: Some(root.to_string_lossy().into_owned()),
            },
            HtmlPageFlags {
                chromeless: true,
                javascript: true,
                ..HtmlPageFlags::default()
            },
        ))
    }

    /// A chromeless file page shown here for another machine, whose files come from `origin`.
    fn replica(origin: &Rc<FakeOrigin>, javascript: bool) -> HtmlOverlay {
        html_overlay::set_replica_link(Rc::clone(origin) as Rc<dyn ReplicaLink>);
        let mut overlay = HtmlOverlay::new(
            HtmlSource::Replica {
                path: "/docs/index.html".to_owned(),
            },
            HtmlPageFlags {
                chromeless: true,
                javascript,
                ..HtmlPageFlags::default()
            },
        );
        overlay.replica_of = Some(Uuid::new_v4());
        overlay
    }

    /// Turns until `done` holds, or fails naming `what`.
    fn wait(&mut self, what: &str, mut done: impl FnMut() -> bool) -> Result<(), String> {
        let deadline = Instant::now() + WAIT;
        while !done() {
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {what}"));
            }
            self.frame()?;
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    /// Clicks the window at `position`, as the platform would: a move there, a press and a release.
    fn click(&mut self, position: gpui_kit::Point<gpui_kit::Pixels>) -> Result<(), String> {
        let handle = self.window.into();
        self.cx
            .update_window(handle, |_, window, cx| {
                let events = [
                    gpui_kit::MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: gpui_kit::Modifiers::default(),
                    }
                    .to_platform_input(),
                    gpui_kit::MouseDownEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        modifiers: gpui_kit::Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    gpui_kit::MouseUpEvent {
                        button: gpui_kit::MouseButton::Left,
                        position,
                        modifiers: gpui_kit::Modifiers::default(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                ];
                for event in events {
                    window.dispatch_event(event, cx);
                }
            })
            .map_err(|error| format!("clicking: {error:#}"))
    }

    /// Mounts `overlay` in a new window `width` x `height`, made the fixture's, and shows it there.
    fn mount(&mut self, overlay: &HtmlOverlay, width: f32, height: f32) -> Result<Entity<PageView>, String> {
        // a window of its own at the page's size
        let window = self
            .cx
            .open_window(size(px(width), px(height)), |_, cx| cx.new(|_| Stage { element: None }))
            .map_err(|error| format!("the window did not open: {error:#}"))?;
        self.window = window;
        let element = self
            .cx
            .update(|cx| {
                window.update(cx, |stage, window, cx| {
                    let element = html_overlay::mount(overlay, None, window, cx);
                    stage.element.clone_from(&element);
                    cx.notify();
                    element
                })
            })
            .map_err(|error| format!("the window is gone: {error:#}"))?
            .ok_or("the build mounts no page")?;
        self.cx.update(|cx| html_overlay::apply(overlay, true, cx));
        Ok(element)
    }

    /// Runs GPUI's tasks and renders a frame.
    fn render(&mut self) -> Result<(), String> {
        self.cx.run_until_parked();
        let handle = self.window.into();
        self.cx
            .update_window(handle, |_, window, cx| window.render_frame(cx))
            .map_err(|error| format!("rendering: {error:#}"))
    }

    /// Runs GPUI's tasks and renders a frame, read back as RGBA rows.
    fn frame(&mut self) -> Result<(u32, Vec<u8>), String> {
        self.render()?;
        let handle = self.window.into();
        let image = self
            .cx
            .capture_screenshot(handle)
            .map_err(|error| format!("reading back: {error:#}"))?;
        Ok((image.width(), image.into_raw()))
    }

    fn stats(&mut self, element: &Entity<PageView>) -> FrameStats {
        self.cx.update(|cx| element.read(cx).frame_stats())
    }

    /// Turns until `done` holds over the stats and the frame read back, or fails naming `what`.
    fn until(
        &mut self,
        what: &str,
        element: &Entity<PageView>,
        mut done: impl FnMut(FrameStats, u32, &[u8]) -> bool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + WAIT;
        loop {
            let (width, pixels) = self.frame()?;
            let stats = self.stats(element);
            if done(stats, width, &pixels) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {what} (frames: {stats:?})"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The RGBA pixel at (`x`, `y`) of a frame `width` wide.
fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    pixels
        .get(at..at + 4)
        .map_or([0; 4], |rgba| [rgba[0], rgba[1], rgba[2], rgba[3]])
}

fn close(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual.iter().zip(expected).all(|(&a, e)| a.abs_diff(e) <= 6)
}

/// Whether the engine renders dmabufs here: GPUI's device names a render node and imports a drawn format.
fn dmabuf_mode() -> bool {
    matches!(
        negotiate(gpui_kit::dmabuf_import_caps().as_deref()),
        FrameSource::DmaBuf { .. }
    )
}

// MARK: - cases

/// The engine renders where the renderer imports: dmabufs on a Mesa GPU (its render node), shared memory on lavapipe,
/// which names none.
fn mode(_: &mut Fixture, gpu: &Gpu) -> Result<(), String> {
    let dmabuf = dmabuf_mode();
    match (gpu.name.as_str(), dmabuf) {
        ("mesa", true) | ("lavapipe", false) => Ok(()),
        (name, dmabuf) => Err(format!("{name} negotiated dmabuf={dmabuf}")),
    }
}

/// A page's frames come back through the element: its color on screen, in the GPU's mode, without a fall-back.
fn loads(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let element = fixture.show(
        "<html><body style='margin:0;background:#c03030'></body></html>",
        320.,
        240.,
    )?;
    fixture.until("the page's color on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 100), RED)
    })?;
    let stats = fixture.stats(&element);
    if dmabuf_mode() {
        check(stats.dmabuf > 0 && stats.shm == 0 && !stats.fell_back, || {
            format!("dmabufs import here, yet the page drew {stats:?}")
        })
    } else {
        check(stats.shm > 0 && stats.dmabuf == 0, || {
            format!("no render node here, yet the page drew {stats:?}")
        })
    }
}

/// A page whose session moved to another window (mounted again from there) gets an element of that window, which the
/// engine draws anew.
fn moved(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let overlay = fixture.page("<html><body style='margin:0;background:#c03030'></body></html>")?;
    let first = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page in its first window", &first, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 100), RED)
    })?;
    let second = fixture.mount(&overlay, 200., 160.)?;
    check(first != second, || {
        "the second window reused the first window's element".to_owned()
    })?;
    fixture.until("the page in its second window", &second, |stats, width, pixels| {
        stats.dmabuf + stats.shm > 0 && close(pixel(pixels, width, width / 2, 80), RED)
    })
}

/// Presents an animation's pace is measured over: a count, not a period, so a loaded host lengthens the measure
/// instead of thinning it.
const PRESENTS: u64 = 40;
/// The engine's display refresh (WPE's default screen): `requestAnimationFrame` never runs faster.
const REFRESH_HZ: f64 = 60.;

/// A canvas animation at 1080p stays on dmabufs (the path video takes), never falls back to SHM, and keeps pace with
/// the presenter: every frame is answered, and the engine draws the next one within the present that follows, so
/// all but a few presents (each a 1080p read-back here, the headless counterpart of a window's present) show a new
/// frame. On lavapipe it keeps running in SHM at a small size.
///
/// The pace is the frames over the presents of the same run, capped by the refresh, never an absolute rate: the
/// presents themselves (a debug build's read-back on the test's own thread) are what a loaded host slows, by 20 ms to
/// 300 ms each here, and with them every frame the engine may draw.
fn animation(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let dmabuf = dmabuf_mode();
    let (width, height) = if dmabuf { (1920., 1080.) } else { (480., 270.) };
    let element = fixture.show(
        "<html><body style='margin:0'><canvas id=c></canvas><script>\
         const c = document.getElementById('c'); c.width = innerWidth; c.height = innerHeight;\
         const g = c.getContext('2d'); let n = 0;\
         function draw() { n++; g.fillStyle = n % 2 ? '#c03030' : '#3030c0'; g.fillRect(0, 0, c.width, c.height);\
         g.fillStyle = '#fff'; g.fillText(String(n), 10, 20); requestAnimationFrame(draw); }\
         requestAnimationFrame(draw);</script></body></html>",
        width,
        height,
    )?;
    fixture.until("the animation's first frames", &element, |stats, _, _| {
        stats.dmabuf + stats.shm > 3
    })?;
    let before = fixture.stats(&element);
    let started = Instant::now();
    // every present read back: the headless renderer submits (and so completes frames) only for a read-back
    for _ in 0..PRESENTS {
        fixture.frame()?;
    }
    let elapsed = started.elapsed().as_secs_f64();
    let after = fixture.stats(&element);
    let frames = (after.dmabuf + after.shm) - (before.dmabuf + before.shm);
    #[expect(clippy::cast_precision_loss, reason = "frame and present counts")]
    let (pace, fps) = {
        let possible = (PRESENTS as f64).min((elapsed * REFRESH_HZ).ceil());
        (frames as f64 / possible, frames as f64 / elapsed)
    };
    println!(
        "    {width}x{height}: {frames} frames in {PRESENTS} presents ({pace:.2} of possible), {fps:.1} frames/s, {after:?}"
    );
    check((after.dmabuf + after.shm).saturating_sub(after.answered) <= 1, || {
        format!("frames handed over were never answered: {after:?}")
    })?;
    if dmabuf {
        check(!after.fell_back && after.shm == 0, || {
            format!("SHM was chosen while dmabufs import: {after:?}")
        })?;
        // 0.93 to 1.0 here at load averages of 19 to 70 (Intel, a debug build); SHM manages 2.5 frames/s at 1080p
        check(pace >= 0.75, || {
            format!("{frames} frames in {PRESENTS} presents over {elapsed:.1} s at 1080p on dmabufs")
        })
    } else {
        // 0.2 to 0.7 here: lavapipe draws both sides on the CPU, and the engine copies each frame out
        check(after.dmabuf == 0 && pace >= 0.125, || {
            format!("{frames} frames in {PRESENTS} presents over {elapsed:.1} s in SHM, {after:?}")
        })
    }
}

// MARK: - pages shown for another machine

/// The machine that opened a replica page, as its channel answers (`ngterm_core::remote::page_serve` on the far side):
/// files from a map, every bridge request answered with one reply, and every request recorded.
pub struct FakeOrigin {
    files: HashMap<&'static str, (&'static str, Vec<u8>)>,
    reply: &'static str,
    requested: RefCell<Vec<String>>,
    bridged: RefCell<Vec<String>>,
    /// A path read but left unanswered, as a large file still on its way.
    held: Option<&'static str>,
    /// How many reads of the held path are answered before the rest are held.
    answered_first: usize,
    parked: RefCell<Vec<ngterm_wpe::SchemeToken>>,
}

impl FakeOrigin {
    fn new(files: &[(&'static str, &'static str, String)]) -> Rc<Self> {
        Rc::new(Self {
            files: files
                .iter()
                .map(|(path, mime, body)| (*path, (*mime, body.clone().into_bytes())))
                .collect(),
            reply: r#"{"ok":true,"result":{"extra":{"nested":"kept"},"text":"done"}}"#,
            requested: RefCell::default(),
            bridged: RefCell::default(),
            held: None,
            answered_first: 0,
            parked: RefCell::default(),
        })
    }
}

impl ReplicaLink for FakeOrigin {
    fn fetch(&self, _: Uuid, path: String, token: ngterm_wpe::SchemeToken) {
        let earlier = self.requested.borrow().iter().filter(|read| **read == path).count();
        if self.held == Some(path.as_str()) && earlier >= self.answered_first {
            self.requested.borrow_mut().push(path);
            self.parked.borrow_mut().push(token);
            return;
        }
        let result = self
            .files
            .get(path.as_str())
            .map(|(mime, body)| PageResource {
                mime_type: (*mime).to_owned(),
                body: body.clone(),
            })
            .ok_or_else(|| "file cannot be read".to_owned());
        self.requested.borrow_mut().push(path);
        html_overlay::answer_resource(token, result);
    }

    fn bridge(&self, _: Uuid, body: String, reply: u64) {
        self.bridged.borrow_mut().push(body);
        html_overlay::answer_value(reply, ngterm_core::html::bridge::shape_raw_reply(self.reply));
    }
}

/// A file replica's page and its stylesheet both come over its channel: the stylesheet's color on screen
/// (`ControlServerRemotePageTests.testAFileReplicaLoadsItsPageAndAnAssetThroughItsChannel`).
fn replica_loads(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let origin = FakeOrigin::new(&[
        (
            "/docs/index.html",
            "text/html",
            "<link rel='stylesheet' href='style.css'><body style='margin:0'><p>x</p></body>".to_owned(),
        ),
        ("/docs/style.css", "text/css", "body { background: #c03030 }".to_owned()),
    ]);
    let overlay = Fixture::replica(&origin, false);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the stylesheet's color on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    let requested = origin.requested.borrow().clone();
    check(requested == ["/docs/index.html", "/docs/style.css"], || {
        format!("the channel was asked for {requested:?}")
    })
}

/// A read naming another page's id fails without reaching the origin (`HtmlPageSchemeHandlerTests.
/// testARequestForAnotherPageNeverReachesTheSource`): the image's failure paints the page.
fn replica_other_page(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let other = Uuid::new_v4();
    let origin = FakeOrigin::new(&[
        (
            "/docs/index.html",
            "text/html",
            format!(
                "<body style='margin:0'><img src='ngterm-page://{other}/docs/data.txt' \
                 onerror=\"document.body.style.background = '#c03030'\"></body>"
            ),
        ),
        ("/docs/data.txt", "text/plain", "data".to_owned()),
    ]);
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the refused read's failure on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    let requested = origin.requested.borrow().clone();
    check(requested == ["/docs/index.html"], || {
        format!("another page's read reached the channel: {requested:?}")
    })
}

/// A read still on its way holds up no other read of the page (`HtmlPageSchemeHandlerTests.
/// testAStoppedRequestCancelsItsSourceAndASlowOneBlocksNoOther`, its second half: `WebKit`'s `GLib` ports tell a scheme
/// handler nothing of a read stopped before its answer, so the first half has no counterpart here).
fn replica_slow(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let mut origin = FakeOrigin::new(&[
        (
            "/docs/index.html",
            "text/html",
            "<body style='margin:0'><script>fetch('slow.bin'); fetch('data.txt').then(r => r.text()).then(t => { \
             if (t === 'data') document.body.style.background = '#c03030'; })</script></body>"
                .to_owned(),
        ),
        ("/docs/data.txt", "text/plain", "data".to_owned()),
    ]);
    Rc::get_mut(&mut origin).ok_or("the origin is shared")?.held = Some("/docs/slow.bin");
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the second read's answer on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    let parked = origin.parked.borrow().len();
    for token in origin.parked.borrow_mut().drain(..) {
        html_overlay::answer_resource(token, Err("cancelled".to_owned()));
    }
    check(parked == 1, || format!("{parked} reads were held"))
}

/// The page's script asks over the bridge; the request goes to the origin as sent and the reply keeps a field this
/// build does not know (`ControlServerRemotePageTests.testThePageScriptsBridgeRelaysToTheOriginAndKeepsUnknownFields`).
fn replica_bridge(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let origin = FakeOrigin::new(&[(
        "/docs/index.html",
        "text/html",
        "<body style='margin:0'><script>ngterm.request('tree').then(r => { \
         if (r.extra && r.extra.nested === 'kept') document.body.style.background = '#c03030'; })</script></body>"
            .to_owned(),
    )]);
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the reply's field on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    let bridged = origin.bridged.borrow().clone();
    check(bridged.len() == 1 && bridged[0].contains(r#""cmd":"tree""#), || {
        format!("the origin got {bridged:?}")
    })
}

/// A tagged control relays through the bridge with the page's own scripts off
/// (`ControlServerRemotePageTests.testATaggedControlRelaysWithPageScriptOff`).
fn replica_tagged(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    let origin = FakeOrigin::new(&[
        (
            "/docs/index.html",
            "text/html",
            "<link rel='stylesheet' href='style.css'><body>\
             <div role='button' id='b' data-ngterm='session.select' data-ngterm-target='3F2A' \
             data-ngterm-into='#out'>go</div><p id='out'></p></body>"
                .to_owned(),
        ),
        (
            "/docs/style.css",
            "text/css",
            "body { margin: 0; background: #c03030 } \
             #b { position: fixed; left: 0; top: 0; width: 200px; height: 150px }"
                .to_owned(),
        ),
    ]);
    let overlay = Fixture::replica(&origin, false);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    fixture.click(point(px(50.), px(50.)))?;
    let seen = Rc::clone(&origin);
    fixture.wait("the tagged control's request at the origin", || {
        !seen.bridged.borrow().is_empty()
    })?;
    let bridged = origin.bridged.borrow().clone();
    check(
        bridged.len() == 1 && bridged[0].contains("session.select") && bridged[0].contains("3F2A"),
        || format!("the origin got {bridged:?}"),
    )
}

/// The `WebView2` host's file-chooser guard (`webview2/bridge.rs`), which no harness runs on Windows, run here in
/// `WebKit` ahead of a page's script: what it refuses is DOM behaviour the engines share. A file input's click that a
/// page dispatches or calls never reaches it, a text input's still does, and a user's click on a file input inside a
/// closed shadow root arrives cancelled. The page paints yellow once its scripted checks hold, green once the click
/// arrived cancelled, blue on a failure.
fn file_chooser_guard(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GUARD: &str = include_str!("../../src/html_overlay/webview2/guard.js");
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let page = format!(
        "<html><body style='margin:0;background:#c03030'>\
         <div id='host' style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <script>{GUARD}</script><script>
           const paint = (color) => {{ document.body.style.background = color; }};
           const root = document.getElementById('host').attachShadow({{mode: 'closed'}});
           root.innerHTML = \"<input type='file' style='position:absolute;left:0;top:0;width:100%;height:100%;opacity:0'>\";
           const detached = document.createElement('input');
           detached.type = 'file';
           let reached = 0;
           detached.addEventListener('click', () => {{ reached += 1; }});
           const dispatched = detached.dispatchEvent(new MouseEvent('click', {{cancelable: true}}));
           detached.click();
           const text = document.createElement('input');
           let typed = 0;
           text.addEventListener('click', () => {{ typed += 1; }});
           text.dispatchEvent(new MouseEvent('click'));
           const scripted = dispatched === false && reached === 0 && typed === 1;
           root.querySelector('input').addEventListener('click', (event) => {{
             paint(scripted && event.defaultPrevented ? '#30c030' : '#3030c0');
           }});
           paint(scripted ? '#c0c030' : '#3030c0');
         </script></body></html>"
    );
    let element = fixture.show(&page, 320., 240.)?;
    let mut seen = [0; 4];
    fixture.until("the page's scripted checks", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, YELLOW) || close(seen, BLUE)
    })?;
    check(close(seen, YELLOW), || {
        "a click the page dispatched or called reached a file input, or a text input's did not".to_owned()
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    fixture.until("the shadowed file input's click", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "a click on a file input inside a closed shadow root was not cancelled".to_owned()
    })
}

/// A second of silence as 16-bit mono PCM WAV at 8 kHz.
fn wav_second() -> Vec<u8> {
    wav_seconds(1)
}

/// `seconds` of silence as 16-bit mono PCM WAV at 8 kHz.
fn wav_seconds(seconds: u32) -> Vec<u8> {
    const RATE: u32 = 8000;
    let data = RATE * 2 * seconds;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2_u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    wav.resize(44 + data as usize, 0);
    wav
}

/// Audio beside a granted file page loads, whose `WebKit` media player reads `http`, `https` and `blob` sources only
/// (`WebKitWebSourceGStreamer.cpp` at WPE `WebKit` 2.54): a `src`, a `<source>`, a fallback past a `<source>` whose
/// file is missing, and a script's `new Audio(url)` each reach their metadata, and no element sees an error. Red once
/// all four loaded, yellow on an error an element saw.
fn granted_audio(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#3030c0'>\
         <audio id=a src='tone.wav' preload=auto></audio>\
         <audio id=b preload=auto><source src='sub/tone.wav' type='audio/wav'></audio>\
         <audio id=d preload=auto><source src='missing.wav'><source src='tone.wav'></audio>\
         <script>
           const loaded = new Set();
           const watch = (el, name) => {
             el.addEventListener('loadedmetadata', () => {
               if (el.duration > 0.9) loaded.add(name);
               if (loaded.size === 4) document.body.style.background = '#c03030';
             });
             el.addEventListener('error', () => { document.body.style.background = '#c0c030'; });
           };
           watch(document.getElementById('a'), 'a');
           watch(document.getElementById('b'), 'b');
           watch(document.getElementById('d'), 'd');
           const c = new Audio('tone.wav');
           c.preload = 'auto';
           watch(c, 'c');
         </script></body>",
        &[("tone.wav", &wav_second()), ("sub/tone.wav", &wav_second())],
    )?;
    let element = fixture.mount(&overlay, 320., 240.)?;
    let mut seen = [0; 4];
    fixture.until("the audio's metadata or an error", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, RED) || close(seen, YELLOW)
    })?;
    check(close(seen, RED), || {
        "the page saw its audio fail to load (GStreamer's base and good plugins installed?)".to_owned()
    })
}

fn check(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition { Ok(()) } else { Err(message()) }
}

/// A `play()` of audio beside a granted file page, which `WebKit` rejects as unsupported before the file is read
/// through the page's scheme, waits for that read and plays: a click (the gesture media needs) plays a script's
/// `new Audio(url)`, and its promise resolves. Green when it does, blue with the rejection's name in the title
/// otherwise.
fn granted_play(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <script>
           document.getElementById('go').addEventListener('click', () => {
             new Audio('tone.wav').play().then(
               () => { document.body.style.background = '#30c030'; },
               (error) => { document.title = error.name; document.body.style.background = '#3030c0'; });
           });
         </script></body>",
        &[("tone.wav", &wav_second())],
    )?;
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    let mut seen = [0; 4];
    fixture.until("the play's outcome", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the page's play() of its audio was rejected".to_owned()
    })
}

/// A replica page's audio, whose rescue's read through the channel is still on its way, is replaced by the page: the
/// second file's failure is rescued too (not dropped behind the first read), and the first read's late failure is not
/// reported on the element that moved on. `WebKit` reads a file of no declared type once itself (to sniff it) before
/// it fails as unsupported, so the origin answers that read and holds the next, the rescue's. Red once the second
/// file loaded, yellow on an error the element saw.
fn replica_audio_replaced(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    let mut origin = FakeOrigin::new(&[(
        "/docs/index.html",
        "text/html",
        "<body style='margin:0;background:#3030c0'><script>
           const a = new Audio('slow.wav');
           a.preload = 'auto';
           a.addEventListener('loadedmetadata', () => {
             if (a.duration > 0.9) document.body.style.background = '#c03030';
           });
           a.addEventListener('error', () => { document.body.style.background = '#c0c030'; });
           // the first file failed as unsupported (the rescue's read is under way, held by the origin): replace it
           const replace = () => (a.error && a.error.code === 4 ? (a.src = 'tone.wav') : setTimeout(replace, 10));
           replace();
         </script></body>"
            .to_owned(),
    )]);
    let fake = Rc::get_mut(&mut origin).ok_or("the origin is shared")?;
    fake.held = Some("/docs/slow.wav");
    fake.answered_first = 1;
    fake.files.insert("/docs/slow.wav", ("audio/wav", wav_second()));
    fake.files.insert("/docs/tone.wav", ("audio/wav", wav_second()));
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    let mut seen = [0; 4];
    fixture.until(
        "the replacement's metadata or an error",
        &element,
        |_, width, pixels| {
            seen = pixel(pixels, width, width / 2, 200);
            close(seen, RED) || close(seen, YELLOW)
        },
    )?;
    check(close(seen, RED), || {
        "the replacement's failure was dropped behind the first read, or the element saw an error".to_owned()
    })?;
    let parked: Vec<_> = origin.parked.borrow_mut().drain(..).collect();
    check(parked.len() == 1, || format!("{} reads were held", parked.len()))?;
    for token in parked {
        html_overlay::answer_resource(token, Err("file cannot be read".to_owned()));
    }
    // the late failure has nothing to say about the element: it stays red
    let settle = Instant::now() + Duration::from_millis(500);
    while Instant::now() < settle {
        let (width, pixels) = fixture.frame()?;
        seen = pixel(&pixels, width, width / 2, 200);
        if !close(seen, RED) {
            break;
        }
    }
    check(close(seen, RED), || {
        "the first read's late failure was reported on the element that moved on".to_owned()
    })
}

/// A `<source>` whose file was rescued, then given another file by the page (a playlist's next track), plays the new
/// file: the rescue changed the `<source>` in place, so no blob of the old file stays a candidate. Red once the
/// two-second file loaded, yellow when the old one-second file loaded again.
fn granted_source_switched(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#3030c0'>\
         <audio id=e preload=auto><source id=s src='tone.wav'></audio>\
         <script>
           const e = document.getElementById('e');
           let switched = false;
           e.addEventListener('loadedmetadata', () => {
             if (!switched) {
               switched = true;
               document.getElementById('s').src = 'long.wav';
               e.load();
             } else {
               document.body.style.background = e.duration > 1.5 ? '#c03030' : '#c0c030';
             }
           });
         </script></body>",
        &[("tone.wav", &wav_second()), ("long.wav", &wav_seconds(2))],
    )?;
    let element = fixture.mount(&overlay, 320., 240.)?;
    let mut seen = [0; 4];
    fixture.until("the second track's metadata", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, RED) || close(seen, YELLOW)
    })?;
    check(close(seen, RED), || "the old track played after the switch".to_owned())
}

/// A `play()` of a file the page's scheme cannot read rejects, after the one failure the page is told about, and a
/// `play()` the page retries from that failure only rejects too, as a browser's does: no new read, no new failure.
/// A click plays it (the gesture media needs); green when the page saw exactly one failure, blue otherwise.
fn granted_unreadable_play(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <script>
           let failures = 0;
           document.getElementById('go').addEventListener('click', () => {
             const a = new Audio('missing.wav');
             a.addEventListener('error', () => {
               failures += 1;
               if (failures < 5) a.play().catch(() => {});
             });
             a.play().catch(() => {});
             setTimeout(() => { document.body.style.background = failures === 1 ? '#30c030' : '#3030c0'; }, 1500);
           });
         </script></body>",
        &[],
    )?;
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    let mut seen = [0; 4];
    fixture.until("the failures counted", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the page saw no failure, or a retried play() read the file and failed again".to_owned()
    })
}

/// A `play()` of an element whose candidates both need a rescue: the first file's blob cannot be decoded, and the
/// second's read is held by the origin, then answered. Each rescue runs the selection again, aborting the play each
/// time; the page's promise still resolves once the second file plays. Green when it does, blue when it rejected.
fn replica_play_reruns(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let mut origin = FakeOrigin::new(&[(
        "/docs/index.html",
        "text/html",
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <audio id=d><source src='bad.wav'><source src='tone.wav'></audio>\
         <script>
           document.getElementById('go').addEventListener('click', () => {
             document.getElementById('d').play().then(
               () => { document.body.style.background = '#30c030'; },
               () => { document.body.style.background = '#3030c0'; });
           });
         </script></body>"
            .to_owned(),
    )]);
    let fake = Rc::get_mut(&mut origin).ok_or("the origin is shared")?;
    fake.files
        .insert("/docs/bad.wav", ("audio/wav", b"not a wave file".to_vec()));
    fake.files.insert("/docs/tone.wav", ("audio/wav", wav_second()));
    fake.held = Some("/docs/tone.wav");
    // WebKit's own sniffing read is answered; the rescue's is held
    fake.answered_first = 1;
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    let held = Rc::clone(&origin);
    fixture.wait("the second file's rescue read", || !held.parked.borrow().is_empty())?;
    // let the first file's blob fail and its rerun of the selection end before the second file arrives
    let settle = Instant::now() + Duration::from_millis(500);
    while Instant::now() < settle {
        fixture.frame()?;
    }
    let parked: Vec<_> = origin.parked.borrow_mut().drain(..).collect();
    for token in parked {
        html_overlay::answer_resource(
            token,
            Ok(PageResource {
                mime_type: "audio/wav".to_owned(),
                body: wav_second(),
            }),
        );
    }
    let mut seen = [0; 4];
    fixture.until("the play's outcome", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the page's play() was rejected by a rerun of the selection".to_owned()
    })
}

/// A `pause()` while a `play()` waits on its rescue, whose read through the channel the origin holds, aborts that
/// play as a browser's pause steps do: the promise rejects with `AbortError`, and once the file arrives the element
/// loads it but stays paused. A first click plays (the gesture media needs), a second pauses.
fn replica_pause_during_rescue(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    pause_and_rescue(
        fixture,
        Pause {
            autoplay: false,
            with_play: false,
        },
    )
}

/// As [`replica_pause_during_rescue`], of an `autoplay` element: the rescue's load of the blob, which turns `WebKit`'s
/// autoplay back on, does not play what the page paused.
fn replica_pause_during_rescue_autoplay(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    pause_and_rescue(
        fixture,
        Pause {
            autoplay: true,
            with_play: false,
        },
    )
}

/// An `autoplay` element the page plays and pauses in one click, before its file fails and its rescue starts, stays
/// paused once the rescue's blob loads.
fn replica_pause_before_rescue_autoplay(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    pause_and_rescue(
        fixture,
        Pause {
            autoplay: true,
            with_play: true,
        },
    )
}

/// How a pause case's page stops its audio.
#[derive(Clone, Copy)]
struct Pause {
    /// The element has `autoplay`.
    autoplay: bool,
    /// The click that plays pauses at once, rather than a second click while the rescue's read is held.
    with_play: bool,
}

/// A replica page's audio, played by a click and paused (see [`Pause`]) while its rescue's read is held by the
/// origin, then given its file. Green when the play was aborted and nothing played, blue otherwise.
fn pause_and_rescue(fixture: &mut Fixture, pause: Pause) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let page = format!(
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <script>
           const a = new Audio('slow.wav');
           a.preload = 'auto';
           a.autoplay = {autoplay};
           const withPlay = {with_play};
           let clicks = 0;
           let outcome = 'pending';
           let playing = false;
           a.addEventListener('playing', () => {{ playing = true; }});
           a.addEventListener('loadedmetadata', () => setTimeout(() => {{
             const aborted = outcome === 'AbortError' && !playing && a.paused;
             document.body.style.background = aborted ? '#30c030' : '#3030c0';
           }}, 500));
           document.getElementById('go').addEventListener('click', () => {{
             clicks += 1;
             if (clicks === 1) {{
               a.play().then(() => {{ outcome = 'resolved'; }}, (error) => {{ outcome = error.name; }});
               if (withPlay) a.pause();
             }} else {{
               a.pause();
             }}
           }});
         </script></body>",
        autoplay = pause.autoplay,
        with_play = pause.with_play,
    );
    let mut origin = FakeOrigin::new(&[("/docs/index.html", "text/html", page)]);
    let fake = Rc::get_mut(&mut origin).ok_or("the origin is shared")?;
    fake.files.insert("/docs/slow.wav", ("audio/wav", wav_second()));
    fake.held = Some("/docs/slow.wav");
    // WebKit's own sniffing read is answered; the rescue's is held
    fake.answered_first = 1;
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the page on screen", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), RED)
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    let held = Rc::clone(&origin);
    fixture.wait("the rescue's read", || !held.parked.borrow().is_empty())?;
    if !pause.with_play {
        fixture.click(point(px(160.), px(120.)))?;
    }
    // let the pause reach the page before the file does
    let settle = Instant::now() + Duration::from_millis(300);
    while Instant::now() < settle {
        fixture.frame()?;
    }
    let parked: Vec<_> = origin.parked.borrow_mut().drain(..).collect();
    for token in parked {
        html_overlay::answer_resource(
            token,
            Ok(PageResource {
                mime_type: "audio/wav".to_owned(),
                body: wav_second(),
            }),
        );
    }
    let mut seen = [0; 4];
    fixture.until("the play's outcome", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the play the page paused resolved, or the audio played once the file arrived".to_owned()
    })
}

/// The failure of a `<source>` a selection only reached because an earlier one failed as unsupported for a moment is
/// not told once that earlier one is rescued and loads: a browser never reaches it, and a page reading an error on its
/// last `<source>` as "nothing plays" would tear down a working player. A first `<source>` that fails is still told,
/// once. Red when the element with the missing last `<source>` loaded silently and the one with the missing first
/// `<source>` told it once, yellow otherwise.
fn granted_later_source_silent(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#3030c0'>\
         <audio id=d preload=auto><source src='tone.wav'><source id=late src='missing.wav'></audio>\
         <audio id=f preload=auto><source id=first src='missing.wav'><source src='tone.wav'></audio>\
         <script>
           let late = 0;
           let first = 0;
           let loaded = 0;
           document.getElementById('late').addEventListener('error', () => { late += 1; });
           document.getElementById('first').addEventListener('error', () => { first += 1; });
           const done = () => {
             loaded += 1;
             if (loaded < 2) return;
             setTimeout(() => {
               document.body.style.background = late === 0 && first === 1 ? '#c03030' : '#c0c030';
             }, 1000);
           };
           for (const id of ['d', 'f']) {
             document.getElementById(id).addEventListener('loadedmetadata', done, { once: true });
           }
         </script></body>",
        &[("tone.wav", &wav_second())],
    )?;
    let element = fixture.mount(&overlay, 320., 240.)?;
    let mut seen = [0; 4];
    fixture.until("both elements loaded and settled", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, RED) || close(seen, YELLOW)
    })?;
    check(close(seen, RED), || {
        "the last <source>'s failure was told while the first played, or the first <source>'s was not told once"
            .to_owned()
    })
}

/// A later `<source>`'s failed read, held back while an earlier one's read is on its way (held by the origin), is told
/// once the page removes that earlier one and its read lands: it holds nothing back in the element any more. A click
/// removes it. Green when the failure was told once, after the removal; blue when it was told before; a timeout when
/// never.
fn replica_held_failure_told_on_removal(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    let mut origin = FakeOrigin::new(&[(
        "/docs/index.html",
        "text/html",
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <audio id=d preload=auto><source id=first src='slow.wav'><source id=last src='missing.wav'></audio>\
         <script>
           let removed = false;
           let told = 0;
           document.getElementById('last').addEventListener('error', () => {
             told += 1;
             document.body.style.background = removed && told === 1 ? '#30c030' : '#3030c0';
           });
           document.getElementById('go').addEventListener('click', () => {
             removed = true;
             document.getElementById('first').remove();
           });
         </script></body>"
            .to_owned(),
    )]);
    let fake = Rc::get_mut(&mut origin).ok_or("the origin is shared")?;
    fake.files.insert("/docs/slow.wav", ("audio/wav", wav_second()));
    fake.held = Some("/docs/slow.wav");
    // WebKit's own sniffing read is answered; the rescue's is held
    fake.answered_first = 1;
    let overlay = Fixture::replica(&origin, true);
    let element = fixture.mount(&overlay, 320., 240.)?;
    let held = Rc::clone(&origin);
    fixture.wait("the first source's rescue read", || !held.parked.borrow().is_empty())?;
    // let the last source's read fail, held back behind the first
    let settle = Instant::now() + Duration::from_millis(500);
    while Instant::now() < settle {
        fixture.frame()?;
    }
    let (width, pixels) = fixture.frame()?;
    check(close(pixel(&pixels, width, width / 2, 200), RED), || {
        "the last source's failure was told while the first one was still being read".to_owned()
    })?;
    fixture.click(point(px(160.), px(120.)))?;
    let settle = Instant::now() + Duration::from_millis(300);
    while Instant::now() < settle {
        fixture.frame()?;
    }
    let parked: Vec<_> = origin.parked.borrow_mut().drain(..).collect();
    for token in parked {
        html_overlay::answer_resource(
            token,
            Ok(PageResource {
                mime_type: "audio/wav".to_owned(),
                body: wav_second(),
            }),
        );
    }
    let mut seen = [0; 4];
    fixture.until("the last source's failure told", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the last source's failure was told more than once".to_owned()
    })
}

/// A file the page's scheme could not read, told to the page once, then put in place: the page's `load()` and
/// `play()` of it read it afresh, and the play resolves (it is not rejected as already unreadable while the audio
/// plays). The click is the gesture media needs. Green when the play resolved, blue when it rejected.
fn granted_unreadable_reloaded(fixture: &mut Fixture, _: &Gpu) -> Result<(), String> {
    const GREEN: [u8; 4] = [0x30, 0xc0, 0x30, 0xff];
    const BLUE: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
    const YELLOW: [u8; 4] = [0xc0, 0xc0, 0x30, 0xff];
    let overlay = fixture.granted_page(
        "index.html",
        "<body style='margin:0;background:#c03030'>\
         <div id=go style='position:fixed;left:0;top:0;width:100%;height:100%'></div>\
         <script>
           const a = new Audio('later.wav');
           a.preload = 'auto';
           a.addEventListener('error', () => { document.body.style.background = '#c0c030'; }, { once: true });
           document.getElementById('go').addEventListener('click', () => {
             a.load();
             a.play().then(
               () => { document.body.style.background = '#30c030'; },
               () => { document.body.style.background = '#3030c0'; });
           });
         </script></body>",
        &[],
    )?;
    let HtmlSource::File {
        grant_root: Some(root), ..
    } = &overlay.source
    else {
        return Err("the granted page has no root".to_owned());
    };
    let root = PathBuf::from(root);
    let element = fixture.mount(&overlay, 320., 240.)?;
    fixture.until("the missing file's failure told", &element, |_, width, pixels| {
        close(pixel(pixels, width, width / 2, 200), YELLOW)
    })?;
    std::fs::write(root.join("later.wav"), wav_second()).map_err(|err| err.to_string())?;
    fixture.click(point(px(160.), px(120.)))?;
    let mut seen = [0; 4];
    fixture.until("the play's outcome", &element, |_, width, pixels| {
        seen = pixel(pixels, width, width / 2, 200);
        close(seen, GREEN) || close(seen, BLUE)
    })?;
    check(close(seen, GREEN), || {
        "the play after the reload was rejected as already unreadable".to_owned()
    })
}
