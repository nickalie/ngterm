//! Real Wry delegate/script integration, without initializing GPUI or requiring a Metal GPU.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
#[expect(
    dead_code,
    unused_imports,
    reason = "this main-thread harness uses shared helpers without registering their embedded unit tests"
)]
#[path = "../src/html_overlay/webview2/bridge.rs"]
mod bridge;

#[cfg(target_os = "macos")]
#[expect(
    clippy::unwrap_used,
    reason = "a failed operation must abort this bounded native regression harness"
)]
mod smoke {
    use super::bridge;
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
    use std::{
        ffi::{CStr, CString, c_char, c_void},
        ptr::NonNull,
        sync::atomic::{AtomicBool, AtomicPtr, Ordering},
        time::{Duration, Instant},
    };
    use wry::WebViewExtMacOS as _;
    static PAGE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static LOADED: AtomicBool = AtomicBool::new(false);
    static REPLIED: AtomicBool = AtomicBool::new(false);
    static NONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    unsafe extern "C" {
        fn ngterm_web_attach(
            view: *mut c_void,
            context: *mut c_void,
            decide: extern "C" fn(*mut c_void, *const c_char, bool, bool, bool, u64) -> bool,
            report: extern "C" fn(*mut c_void, *const c_char),
            key: extern "C" fn(*mut c_void, u32) -> bool,
        ) -> *mut c_void;
        fn ngterm_web_detach(delegate: *mut c_void);
        fn ngterm_web_reply(view: *mut c_void, json: *const c_char);
        fn ngterm_web_document_script(view: *mut c_void, script: *const c_char, world: *const c_char);
    }
    extern "C" fn allow(_: *mut c_void, _: *const c_char, _: bool, _: bool, _: bool, _: u64) -> bool {
        true
    }
    extern "C" fn key(_: *mut c_void, _: u32) -> bool {
        false
    }
    extern "C" fn report(_: *mut c_void, json: *const c_char) {
        // SAFETY: native callback lends a NUL-terminated JSON string for this call.
        let json = unsafe { CStr::from_ptr(json) }.to_str().unwrap();
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        if value["kind"] == "complete" && value["success"] == true {
            LOADED.store(true, Ordering::Relaxed);
        }
        if value["kind"] == "message"
            && let Some(bridge::Message::Bridge { id, .. }) =
                bridge::parse(value["body"].as_str().unwrap(), NONCE.get().unwrap())
        {
            let reply = CString::new(bridge::reply(id, NONCE.get().unwrap(), Ok(serde_json::json!(42)))).unwrap();
            // SAFETY: live main-thread view, native handler copies this string synchronously.
            unsafe {
                ngterm_web_reply(PAGE.load(Ordering::Relaxed), reply.as_ptr());
            }
            REPLIED.store(true, Ordering::Relaxed);
        }
    }
    struct Parent(Retained<AnyObject>);
    impl HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let view = NonNull::from(&*self.0).cast();
            // SAFETY: the parent owns this NSView throughout the borrowed handle's lifetime.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(view))) })
        }
    }
    fn pump_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "native WebKit operation timed out");
            // SAFETY: AppKit's main thread; these selectors return live autoreleased Foundation objects.
            let run_loop: *mut AnyObject = unsafe { msg_send![class!(NSRunLoop), currentRunLoop] };
            // SAFETY: NSDate returns an autoreleased date valid throughout this iteration.
            let date: *mut AnyObject = unsafe { msg_send![class!(NSDate), dateWithTimeIntervalSinceNow:0.02f64] };
            // SAFETY: main run loop and live date returned above.
            let _: () = unsafe { msg_send![run_loop,runUntilDate:date] };
        }
    }
    fn isolated_document_renderer(parent: &Parent) {
        let view = wry::WebViewBuilder::new()
            .with_incognito(true)
            .with_javascript_disabled()
            .build_as_child(parent)
            .unwrap();
        let pointer = Retained::as_ptr(&view.webview()).cast_mut().cast();
        let script = CString::new("window.helperOnly=true; document.body.dataset.renderer='isolated'").unwrap();
        let world = CString::new("ngterm-markdown").unwrap();
        // SAFETY: the live main-thread web view copies the helper script into its isolated content world.
        unsafe { ngterm_web_document_script(pointer, script.as_ptr(), world.as_ptr()) };
        view.load_html("<body><script>window.authored=true</script></body>")
            .unwrap();
        let checked = std::sync::Arc::new(AtomicBool::new(false));
        pump_until(|| {
            let result = checked.clone();
            view.evaluate_script_with_callback(
                "document.body.dataset.renderer==='isolated' && typeof window.helperOnly==='undefined' && typeof window.authored==='undefined'",
                move |value| { result.store(value=="true", Ordering::Relaxed); },
            ).unwrap();
            checked.load(Ordering::Relaxed)
        });
        println!("PASS: isolated document renderer with authored scripts disabled");
    }

    pub fn run() {
        // Forces linkage of the crate's native adapter as well as checking that startup is not used.
        assert!(!ngterm_gpui::app::build_identity_text().is_empty());
        // SAFETY: this harness runs on the process's main thread. Cocoa retains the shared application.
        let _app: *mut AnyObject = unsafe { msg_send![class!(NSApplication), sharedApplication] };
        // SAFETY: main thread, retained NSView allocation; the parent outlives the web view.
        let parent = Parent(unsafe { msg_send![class!(NSView), new] });
        let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(640., 480.));
        // SAFETY: main-thread NSWindow allocation, initialized and retained immediately below.
        let allocated: *mut AnyObject = unsafe { msg_send![class!(NSWindow), alloc] };
        // SAFETY: valid frame, titled style and buffered backing; no window is ordered onto the screen.
        let window: *mut AnyObject =
            unsafe { msg_send![allocated,initWithContentRect:rect, styleMask:1usize, backing:2usize, defer:false] };
        // SAFETY: init returned an owned (+1) NSWindow; this transfers that ownership to the retained handle.
        let window = unsafe { Retained::from_raw(window) }.unwrap();
        // SAFETY: the window retains this live NSView; Wry requires its native parent to belong to a window.
        let _: () = unsafe { msg_send![&*window,setContentView:&*parent.0] };
        let nonce = bridge::nonce();
        NONCE.set(nonce.clone()).unwrap();
        let view = wry::WebViewBuilder::new()
            .with_incognito(true)
            .with_ipc_handler(|_| {})
            .with_initialization_script(bridge::helper_script())
            .with_initialization_script(bridge::page_script(&nonce, true, true))
            .with_html("<html><body>bridge regression</body></html>")
            .build_as_child(&parent)
            .unwrap();
        let pointer = Retained::as_ptr(&view.webview()).cast_mut().cast();
        PAGE.store(pointer, Ordering::Relaxed);
        let mut context = 0u8;
        // SAFETY: view and context live until detach; callbacks have static lifetimes.
        let delegate = unsafe { ngterm_web_attach(pointer, (&raw mut context).cast(), allow, report, key) };
        let evaluated = std::sync::Arc::new(AtomicBool::new(false));
        let result = evaluated.clone();
        // This starts in Wry's pending queue. Replacing its delegate used to leave it there forever.
        view.evaluate_script("window.wryQueueFlushed=true").unwrap();
        pump_until(|| LOADED.load(Ordering::Relaxed));
        view.evaluate_script_with_callback("window.wryQueueFlushed===true", move |value| {
            assert_eq!(value, "true");
            result.store(true, Ordering::Relaxed);
        })
        .unwrap();
        pump_until(|| evaluated.load(Ordering::Relaxed));
        view.evaluate_script("window.dispatchEvent=()=>{throw Error('trap')}; window.MessageEvent=()=>{throw Error('trap')}; window.ngterm.request('test').then(answer=>window.bridgeAnswer=answer)").unwrap();
        pump_until(|| REPLIED.load(Ordering::Relaxed));
        let settled = std::sync::Arc::new(AtomicBool::new(false));
        pump_until(|| {
            let result = settled.clone();
            view.evaluate_script_with_callback("window.bridgeAnswer===42", move |value| {
                result.store(value == "true", Ordering::Relaxed);
            })
            .unwrap();
            settled.load(Ordering::Relaxed)
        });
        // SAFETY: detach before dropping view/context, on their owning main thread.
        unsafe {
            ngterm_web_detach(delegate);
        }
        println!("PASS: Wry pending script flush and production bridge promise transport");
        isolated_document_renderer(&parent);
    }
}
#[cfg(target_os = "macos")]
fn main() {
    objc2::rc::autoreleasepool(|_| smoke::run());
}
