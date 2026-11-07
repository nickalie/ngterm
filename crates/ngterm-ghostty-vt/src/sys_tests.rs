use std::process::Command;
use std::sync::Mutex;

use super::*;

static LOGS: Mutex<Vec<(LogLevel, String, String)>> = Mutex::new(Vec::new());

fn decoder(bytes: &[u8]) -> Option<DecodedImage> {
    let rgba = match bytes {
        b"empty" => Vec::new(),
        b"short" => vec![1, 2, 3],
        b"valid" => vec![1, 2, 3, 255],
        _ => return None,
    };
    Some(DecodedImage {
        width: 1,
        height: 1,
        rgba,
    })
}

fn sink(level: LogLevel, scope: &str, message: &str) {
    LOGS.lock().unwrap().push((level, scope.to_owned(), message.to_owned()));
}

#[test]
fn hooks_validate_images_and_deliver_logs_in_an_isolated_process() {
    const CASE: &str = "T_GVT_SYS_HOOKS_CASE";
    if let Ok(case) = std::env::var(CASE) {
        assert!(installed().is_none());
        let hooks = if case == "empty" {
            Hooks { png: None, log: None }
        } else {
            Hooks {
                png: Some(decoder),
                log: Some(sink),
            }
        };
        assert!(install(hooks));
        assert!(!install(Hooks { png: None, log: None }));
        if case == "empty" {
            assert!(!decode(b"valid").0);
            emit_logs();
            assert!(LOGS.lock().unwrap().is_empty());
        } else {
            check_images();
            check_logs();
        }
        return;
    }
    for case in ["empty", "hooks"] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "sys::tests::hooks_validate_images_and_deliver_logs_in_an_isolated_process",
            "--nocapture",
        ]);
        command.env(CASE, case);
        for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("NGTERM_")) {
            command.env_remove(key);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn decode(bytes: &[u8]) -> (bool, ffi::GhosttySysImage) {
    let mut image = ffi::GhosttySysImage {
        width: 0,
        height: 0,
        data: std::ptr::null_mut(),
        data_len: 0,
    };
    // SAFETY: input bytes and the output image are live; null selects the library's default allocator.
    let ok = unsafe {
        decode_png(
            std::ptr::null_mut(),
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len(),
            &raw mut image,
        )
    };
    (ok, image)
}

fn check_images() {
    for bytes in [b"missing".as_slice(), b"empty", b"short"] {
        let (ok, image) = decode(bytes);
        assert!(!ok);
        assert!(image.data.is_null());
        assert_eq!((image.width, image.height, image.data_len), (0, 0, 0));
    }
    let (ok, image) = decode(b"valid");
    assert!(ok);
    assert_eq!((image.width, image.height, image.data_len), (1, 1, 4));
    // SAFETY: the successful decoder returned `data_len` bytes allocated by the default allocator.
    let rgba = unsafe { std::slice::from_raw_parts(image.data, image.data_len) };
    assert_eq!(rgba, [1, 2, 3, 255]);
    // SAFETY: this image is owned here and was allocated by the default allocator with `data_len` bytes.
    unsafe { ffi::ghostty_free(std::ptr::null(), image.data, image.data_len) };
}

fn emit_logs() {
    for level in [
        ffi::GHOSTTY_SYS_LOG_LEVEL_ERROR,
        ffi::GHOSTTY_SYS_LOG_LEVEL_WARNING,
        ffi::GHOSTTY_SYS_LOG_LEVEL_INFO,
        ffi::GHOSTTY_SYS_LOG_LEVEL_DEBUG,
    ] {
        // SAFETY: the scope and message bytes are live for this synchronous callback.
        unsafe { log(std::ptr::null_mut(), level, b"vt".as_ptr(), 2, b"bad\xff".as_ptr(), 4) };
    }
}

fn check_logs() {
    emit_logs();
    assert_eq!(
        *LOGS.lock().unwrap(),
        [LogLevel::Error, LogLevel::Warning, LogLevel::Info, LogLevel::Debug].map(|level| (
            level,
            "vt".to_owned(),
            "bad\u{fffd}".to_owned()
        ))
    );
}
