use std::cell::RefCell;
use std::os::fd::{AsRawFd as _, IntoRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glib::translate::from_glib_full;

use super::*;
use crate::config::BufferFormat;
use crate::frames::ShmRegion;

/// Runs `case` (the test `name` of this module) alone in a process of its own, with a main context of its own as the
/// thread's default, as the engine thread has: the view's acknowledgements run from sources on it. `WPEPlatform`'s
/// first display registers types process-wide, which two test threads must not race on.
fn on_engine_context(name: &str, case: impl FnOnce(&glib::MainContext)) {
    crate::tests::isolated(&format!("engine::platform::tests::{name}"), || {
        let context = glib::MainContext::new();
        context.with_thread_default(|| case(&context)).unwrap();
    });
}

/// Runs `context` until `done`, within a deadline.
fn run_until(context: &glib::MainContext, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "the view's acknowledgements did not run");
        context.iteration(false);
    }
}

fn shm_display() -> Display {
    Display::connect(None).unwrap()
}

/// A view of `display`, made the way `WebKit` makes one (its `create_view`).
fn view_of_display(display: &Display) -> View {
    // SAFETY: a live display.
    let raw = unsafe { platform::wpe_view_new(display.as_wpe()) };
    // SAFETY: the view came back as a new reference, which we take over.
    let view: WpeView = unsafe { from_glib_full(raw) };
    view.downcast::<View>().unwrap()
}

/// An ARGB SHM buffer of `width` x `height` whose every pixel is its position (x, y, 0, 255).
fn shm_buffer(display: &Display, width: u8, height: u8) -> WpeBuffer {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[x, y, 0, 255]);
        }
    }
    let bytes = glib::Bytes::from_owned(pixels);
    // SAFETY: a live display and bytes the buffer takes its own reference to.
    let raw = unsafe {
        platform::wpe_buffer_shm_new(
            display.as_wpe(),
            c_int::from(width),
            c_int::from(height),
            platform::WPE_PIXEL_FORMAT_ARGB8888,
            bytes.to_glib_none().0,
            u32::from(width) * 4,
        )
    };
    // SAFETY: a new buffer, which we take over.
    let buffer: WpeBufferShm = unsafe { from_glib_full(raw) };
    buffer.upcast()
}

const XR24: u32 = u32::from_le_bytes(*b"XR24");

/// A 4x2 dmabuf buffer of `display` with one plane over `plane` (any descriptor: the view only duplicates it), at
/// offset 8 and stride 16.
fn dmabuf_buffer(display: &Display, plane: OwnedFd, modifier: u64) -> WpeBuffer {
    let mut fds = [plane.into_raw_fd()];
    let mut offsets = [8u32];
    let mut strides = [16u32];
    // SAFETY: a live display and one plane's arrays; the buffer takes over the descriptor.
    let raw = unsafe {
        platform::wpe_buffer_dma_buf_new(
            display.as_wpe(),
            4,
            2,
            XR24,
            1,
            fds.as_mut_ptr(),
            offsets.as_mut_ptr(),
            strides.as_mut_ptr(),
            modifier,
        )
    };
    // SAFETY: a new buffer, which we take over.
    let buffer: WpeBufferDmaBuf = unsafe { from_glib_full(raw) };
    buffer.upcast()
}

fn rectangle(x: c_int, y: c_int, width: c_int, height: c_int) -> platform::WPERectangle {
    platform::WPERectangle { x, y, width, height }
}

/// Hands `buffer` to `view` as `WebKit` does; the view's error when it refuses it.
fn render(view: &View, buffer: &WpeBuffer, damage: &[platform::WPERectangle]) -> Result<(), glib::Error> {
    let mut error = ptr::null_mut();
    let count = u32::try_from(damage.len()).unwrap();
    let damage = if damage.is_empty() {
        ptr::null()
    } else {
        damage.as_ptr()
    };
    // SAFETY: a live view and buffer, `count` rectangles at `damage`, and a valid error out-pointer.
    let rendered = unsafe {
        platform::wpe_view_render_buffer(view.as_wpe(), buffer.to_glib_none().0, damage, count, &raw mut error)
    };
    if rendered == glib::ffi::GFALSE {
        // SAFETY: a refused buffer sets the error, which we take over.
        return Err(unsafe { from_glib_full(error) });
    }
    assert!(error.is_null());
    Ok(())
}

/// Tells `view` its buffer set is `buffers`, as `WebKit` does.
fn change_buffers(view: &View, buffers: &[&WpeBuffer]) {
    let mut raw: Vec<*mut platform::WPEBuffer> = buffers.iter().map(|buffer| buffer.to_glib_none().0).collect();
    let count = u32::try_from(raw.len()).unwrap();
    // SAFETY: a live view and `count` live buffers.
    unsafe { platform::wpe_view_buffers_changed(view.as_wpe(), raw.as_mut_ptr(), count) };
}

/// Sets `view`'s cursor by name, as `WebKit` does.
fn set_cursor(view: &View, name: &str) {
    let name = CString::new(name).unwrap();
    // SAFETY: a live view and a NUL-terminated name.
    unsafe { platform::wpe_view_set_cursor_from_name(view.as_wpe(), name.as_ptr()) };
}

/// What `WebKit` hears back from `view`: each buffer given back as rendered or released, in order.
fn acknowledgements(view: &View) -> Rc<RefCell<Vec<(&'static str, WpeBuffer)>>> {
    let heard = Rc::new(RefCell::new(Vec::new()));
    for (signal, ack) in [("buffer-rendered", "rendered"), ("buffer-released", "released")] {
        let heard = Rc::clone(&heard);
        view.connect_local(signal, false, move |values| {
            heard.borrow_mut().push((ack, values[1].get::<WpeBuffer>().unwrap()));
            None
        });
    }
    heard
}

/// A view of a new SHM display attached to page 7: the display, the view and the page's events.
fn page_view() -> (Display, View, async_channel::Receiver<Event>) {
    let display = shm_display();
    let view = view_of_display(&display);
    let (events, received) = async_channel::unbounded();
    view.attach(Link { id: ViewId(7), events });
    (display, view, received)
}

fn frame(events: &async_channel::Receiver<Event>) -> Frame {
    match events.try_recv() {
        Ok(Event::Frame(frame)) => frame,
        other => panic!("not a frame: {other:?}"),
    }
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect { x, y, width, height }
}

/// `(st_dev, st_ino)` of `fd`: what tells two descriptors of one file apart from two files.
fn inode(fd: &impl std::os::fd::AsFd) -> (u64, u64) {
    let stat = rustix::fs::fstat(fd).unwrap();
    (stat.st_dev, stat.st_ino)
}

/// `display`'s EGL display, or the error it gives instead.
fn egl_display(display: &Display) -> Result<glib::ffi::gpointer, glib::Error> {
    let mut error = ptr::null_mut();
    // SAFETY: a live display and a valid error out-pointer.
    let egl = unsafe { platform::wpe_display_get_egl_display(display.as_wpe(), &raw mut error) };
    if egl.is_null() {
        // SAFETY: a display without EGL sets the error, which we take over.
        return Err(unsafe { from_glib_full(error) });
    }
    Ok(egl)
}

#[test]
fn an_shm_display_has_no_egl_display_device_or_formats_and_a_render_node_with_a_nul_is_refused() {
    on_engine_context(
        "an_shm_display_has_no_egl_display_device_or_formats_and_a_render_node_with_a_nul_is_refused",
        |_| {
            let display = shm_display();
            assert!(!display.renders_dmabuf());
            assert_eq!(
                egl_display(&display).unwrap_err().message(),
                "SHM frames: the display renders without EGL"
            );
            // SAFETY: a live display.
            assert!(unsafe { platform::wpe_display_get_drm_device(display.as_wpe()) }.is_null());
            // SAFETY: as above.
            assert!(unsafe { platform::wpe_display_get_preferred_buffer_formats(display.as_wpe()) }.is_null());
            let target = DmaBufTarget {
                render_node: PathBuf::from(std::ffi::OsStr::from_bytes(b"/dev/dri/render\0D128")),
                formats: Vec::new(),
            };
            assert_eq!(
                Display::connect(Some(target)).unwrap_err(),
                "the render node path holds a NUL byte"
            );
        },
    );
}

#[test]
fn a_view_without_a_page_hands_each_frame_straight_back_and_follows_its_toplevel() {
    on_engine_context(
        "a_view_without_a_page_hands_each_frame_straight_back_and_follows_its_toplevel",
        |context| {
            let display = shm_display();
            let view = view_of_display(&display);
            let heard = acknowledgements(&view);
            let buffer = shm_buffer(&display, 2, 2);
            render(&view, &buffer, &[]).unwrap();
            assert!(
                heard.borrow().is_empty(),
                "never acknowledged inside WebKit's own hand-over"
            );
            run_until(context, || heard.borrow().len() == 2);
            assert_eq!(*heard.borrow(), [("rendered", buffer.clone()), ("released", buffer)]);

            // SAFETY: a live display.
            let raw = unsafe { platform::wpe_display_create_toplevel(display.as_wpe(), 1) };
            // SAFETY: the toplevel came back as a new reference, which we take over.
            let toplevel: WpeToplevel = unsafe { from_glib_full(raw) };
            let toplevel = toplevel.downcast::<Toplevel>().unwrap();
            let mapped = || {
                // SAFETY: a live view.
                let mapped = unsafe { platform::wpe_view_get_mapped(view.as_wpe()) };
                mapped != glib::ffi::GFALSE
            };
            // SAFETY: a live view and toplevel.
            unsafe { platform::wpe_view_set_toplevel(view.as_wpe(), toplevel.as_wpe()) };
            assert!(mapped(), "a view with a toplevel maps");
            toplevel.set_geometry(30, 20, 2.0);
            // SAFETY: a live view.
            let width = unsafe { platform::wpe_view_get_width(view.as_wpe()) };
            // SAFETY: as above.
            let height = unsafe { platform::wpe_view_get_height(view.as_wpe()) };
            assert_eq!((width, height), (30, 20));
            // SAFETY: as above.
            let scale = unsafe { platform::wpe_view_get_scale(view.as_wpe()) };
            assert!((scale - 2.0).abs() < f64::EPSILON);
            // SAFETY: a live view; no toplevel.
            unsafe { platform::wpe_view_set_toplevel(view.as_wpe(), ptr::null_mut()) };
            assert!(!mapped(), "a view that loses its toplevel unmaps");
            assert!(view.toplevel().is_none());
        },
    );
}

#[test]
fn a_page_s_shm_frame_carries_its_damaged_pixels_and_its_buffer_goes_back_at_once() {
    on_engine_context(
        "a_page_s_shm_frame_carries_its_damaged_pixels_and_its_buffer_goes_back_at_once",
        |context| {
            let (display, view, received) = page_view();
            let heard = acknowledgements(&view);
            let buffer = shm_buffer(&display, 4, 2);
            // an empty rectangle is no damage, and one past the buffer's edge is cut there in its pixels
            let damage = [
                rectangle(0, 0, 2, 1),
                rectangle(0, 0, 0, 1),
                rectangle(1, 0, 1, -1),
                rectangle(2, 1, 5, 1),
            ];
            render(&view, &buffer, &damage).unwrap();
            let shown = frame(&received);
            assert_eq!((shown.view, shown.width, shown.height), (ViewId(7), 4, 2));
            assert_eq!(shown.damage, [rect(0, 0, 2, 1), rect(2, 1, 5, 1)]);
            let FrameContent::Shm(regions) = shown.content else {
                panic!("an SHM frame")
            };
            let expected = [
                ShmRegion {
                    rect: rect(0, 0, 2, 1),
                    pixels: [[0, 0, 0, 255], [1, 0, 0, 255]].concat(),
                },
                ShmRegion {
                    rect: rect(2, 1, 2, 1),
                    pixels: [[2, 1, 0, 255], [3, 1, 0, 255]].concat(),
                },
            ];
            assert_eq!(
                regions, expected,
                "the damaged pixels, copied out and clipped to the buffer"
            );
            // copied out: the buffer goes back at once, and the frame counts as rendered once presented
            run_until(context, || heard.borrow().len() == 1);
            view.presented(BufferId(u64::MAX));
            view.presented(shown.buffer);
            run_until(context, || heard.borrow().len() == 2);
            assert_eq!(*heard.borrow(), [("released", buffer.clone()), ("rendered", buffer)]);
        },
    );
}

#[test]
fn a_page_s_dmabuf_frame_carries_the_host_s_own_planes_shared_by_every_frame_of_its_buffer() {
    on_engine_context(
        "a_page_s_dmabuf_frame_carries_the_host_s_own_planes_shared_by_every_frame_of_its_buffer",
        |_| {
            // an SHM display offers no format: the view hands the buffer on all the same (the consumer checks it)
            let (display, view, received) = page_view();
            let plane = OwnedFd::from(tempfile::tempfile().unwrap());
            let plane_inode = inode(&plane);
            let buffer = dmabuf_buffer(&display, plane, crate::MODIFIER_LINEAR);
            render(&view, &buffer, &[]).unwrap();
            let shown = frame(&received);
            assert!(shown.damage.is_empty());
            let FrameContent::DmaBuf(dmabuf) = shown.content else {
                panic!("a dmabuf frame")
            };
            assert_eq!(
                (dmabuf.width, dmabuf.height, dmabuf.fourcc, dmabuf.modifier),
                (4, 2, XR24, crate::MODIFIER_LINEAR)
            );
            assert_eq!(dmabuf.planes.len(), 1);
            assert_eq!((dmabuf.planes[0].offset, dmabuf.planes[0].stride), (8, 16));
            assert_eq!(inode(&dmabuf.planes[0].fd), plane_inode);
            let raw = buffer.downcast_ref::<WpeBufferDmaBuf>().unwrap().to_glib_none().0;
            // SAFETY: a live dmabuf buffer with one plane.
            let engine_fd = unsafe { platform::wpe_buffer_dma_buf_get_fd(raw, 0) };
            assert_ne!(dmabuf.planes[0].fd.as_raw_fd(), engine_fd, "the host's own duplicate");
            render(&view, &buffer, &[rectangle(0, 0, 1, 1)]).unwrap();
            let again = frame(&received);
            assert_eq!(again.damage, [rect(0, 0, 1, 1)]);
            let FrameContent::DmaBuf(shared) = again.content else {
                panic!("a dmabuf frame")
            };
            assert!(Arc::ptr_eq(&dmabuf, &shared), "one dmabuf per buffer");
        },
    );
}

#[test]
fn a_page_s_view_reports_buffers_leaving_its_set_its_cursor_and_on_closing_the_buffers_it_held() {
    on_engine_context(
        "a_page_s_view_reports_buffers_leaving_its_set_its_cursor_and_on_closing_the_buffers_it_held",
        |_| {
            let (display, view, received) = page_view();
            let (first, second) = (shm_buffer(&display, 4, 2), shm_buffer(&display, 4, 2));
            change_buffers(&view, &[&first, &second]);
            assert!(received.is_empty(), "a first set removes nothing");
            // a buffer outside the set joins it once it brings a frame
            let third = shm_buffer(&display, 4, 2);
            render(&view, &third, &[]).unwrap();
            let third_id = frame(&received).buffer;
            change_buffers(&view, &[&second]);
            assert!(matches!(
                received.try_recv(),
                Ok(Event::BuffersRemoved { view: ViewId(7), buffers }) if buffers == [buffer_id(&first), third_id]
            ));
            set_cursor(&view, "pointer");
            assert!(matches!(
                received.try_recv(),
                Ok(Event::Cursor { view: ViewId(7), name }) if name == "pointer"
            ));
            // the page closes: the buffers it still holds go, and nothing more is reported
            view.detach();
            assert!(matches!(
                received.try_recv(),
                Ok(Event::BuffersRemoved { view: ViewId(7), buffers }) if buffers == [buffer_id(&second)]
            ));
            view.detach();
            set_cursor(&view, "text");
            render(&view, &second, &[]).unwrap();
            assert!(received.is_empty(), "a detached view reports nothing");
        },
    );
}

/// The modifiers of format `format` of group `group` in `formats`.
///
/// # Safety
///
/// `formats` is live and holds that format.
unsafe fn modifiers(formats: *mut platform::WPEBufferFormats, group: u32, format: u32) -> Vec<u64> {
    // SAFETY: the caller's formats and indices; the array of `guint64` is owned by them.
    let array = unsafe { platform::wpe_buffer_formats_get_format_modifiers(formats, group, format) };
    // SAFETY: a live `GArray`.
    let array = unsafe { &*array };
    // SAFETY: `len` elements of 8 bytes at `data`.
    let bytes = unsafe { std::slice::from_raw_parts(array.data.cast::<u8>(), array.len as usize * 8) };
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|word| u64::from_ne_bytes(*word))
        .collect()
}

/// Each format of the one group of `display`'s preferred formats, with its modifiers.
fn offered_formats(display: &Display) -> Vec<(u32, Vec<u64>)> {
    // SAFETY: a live display; the formats are owned by it.
    let offered = unsafe { platform::wpe_display_get_preferred_buffer_formats(display.as_wpe()) };
    assert!(!offered.is_null());
    // SAFETY: live formats.
    assert_eq!(unsafe { platform::wpe_buffer_formats_get_n_groups(offered) }, 1);
    // SAFETY: live formats with one group.
    let count = unsafe { platform::wpe_buffer_formats_get_group_n_formats(offered, 0) };
    (0..count)
        .map(|format| {
            // SAFETY: live formats and a format of their group.
            let fourcc = unsafe { platform::wpe_buffer_formats_get_format_fourcc(offered, 0, format) };
            // SAFETY: as above.
            (fourcc, unsafe { modifiers(offered, 0, format) })
        })
        .collect()
}

#[test]
fn a_display_on_a_render_node_offers_the_consumer_s_formats() {
    let Some(node) = render_node() else {
        eprintln!("render node formats: skipped (no DRM render node this test can open)");
        return;
    };
    on_engine_context("a_display_on_a_render_node_offers_the_consumer_s_formats", |_| {
        let formats = vec![BufferFormat {
            fourcc: XR24,
            modifiers: vec![crate::MODIFIER_LINEAR, crate::MODIFIER_INVALID],
        }];
        let display = Display::connect(Some(DmaBufTarget {
            render_node: node,
            formats,
        }))
        .unwrap();
        assert!(display.renders_dmabuf());
        if let Err(error) = egl_display(&display) {
            panic!("no EGL display on the render node: {error}");
        }
        // SAFETY: a live display; the device is owned by it.
        assert!(!unsafe { platform::wpe_display_get_drm_device(display.as_wpe()) }.is_null());
        assert_eq!(
            offered_formats(&display),
            [(XR24, vec![crate::MODIFIER_LINEAR, crate::MODIFIER_INVALID])]
        );
        let view = view_of_display(&display);
        let (events, received) = async_channel::unbounded();
        view.attach(Link { id: ViewId(2), events });
        let buffer = dmabuf_buffer(
            &display,
            OwnedFd::from(tempfile::tempfile().unwrap()),
            crate::MODIFIER_LINEAR,
        );
        render(&view, &buffer, &[]).unwrap();
        assert!(matches!(frame(&received).content, FrameContent::DmaBuf(_)));
    });
}

/// The first DRM render node this process may open (`WPE_TEST_RENDER_NODE` picks another), as the headless tests
/// pick theirs.
fn render_node() -> Option<PathBuf> {
    if let Some(node) = std::env::var_os("WPE_TEST_RENDER_NODE") {
        return Some(PathBuf::from(node));
    }
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("renderD"))
        })
        .collect();
    nodes.sort();
    nodes
        .into_iter()
        .find(|node| std::fs::OpenOptions::new().read(true).write(true).open(node).is_ok())
}

glib::wrapper! {
    /// A buffer of neither kind the view knows.
    struct ForeignBuffer(ObjectSubclass<foreign_imp::ForeignBuffer>) @extends WpeBuffer;
}

// SAFETY: `WPEBuffer` is a GObject class whose class struct starts with `GObjectClass` (the -sys crate's layout tests
// check it against the header); the subclass adds nothing to the instance or class struct.
unsafe impl<T: ObjectImpl> IsSubclassable<T> for WpeBuffer {}

mod foreign_imp {
    use glib::subclass::prelude::*;

    #[derive(Default)]
    pub(super) struct ForeignBuffer;

    #[glib::object_subclass]
    impl ObjectSubclass for ForeignBuffer {
        const NAME: &'static str = "NgtermWpeTestForeignBuffer";
        type Type = super::ForeignBuffer;
        type ParentType = super::WpeBuffer;
    }

    impl ObjectImpl for ForeignBuffer {}
}

#[test]
fn a_buffer_of_an_unknown_kind_is_refused_with_the_view_s_render_error() {
    on_engine_context(
        "a_buffer_of_an_unknown_kind_is_refused_with_the_view_s_render_error",
        |_| {
            let (display, view, received) = page_view();
            let foreign: ForeignBuffer = glib::Object::builder()
                .property("display", display.upcast_ref::<WpeDisplay>())
                .property("width", 4)
                .property("height", 4)
                .build();
            let error = render(&view, foreign.upcast_ref(), &[]).unwrap_err();
            assert_eq!(error.message(), "a buffer of an unknown kind");
            // SAFETY: a plain quark lookup.
            let domain = unsafe { platform::wpe_view_error_quark() };
            // SAFETY: WPE's view error quark, a registered domain.
            let domain: glib::Quark = unsafe { glib::translate::from_glib(domain) };
            assert_eq!(error.domain(), domain);
            assert!(received.is_empty(), "nothing reaches the page");
        },
    );
}
