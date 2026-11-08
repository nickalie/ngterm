use super::*;
use gpui_kit::AppContext as _;

#[gpui_kit::test]
fn text_and_empty_clipboard_items_stay_on_the_ordinary_paste_path(cx: &mut gpui_kit::TestAppContext) {
    let terminal = cx.new(|cx| {
        Terminal::new(
            super::super::TerminalSpec::default(),
            super::super::TerminalConfig::default(),
            Box::new(|_| panic!("no view starts a pane")),
            cx,
        )
    });
    terminal.update(cx, |terminal, cx| {
        assert!(!start(
            terminal.shared(),
            "unreachable.invalid",
            &ClipboardItem::new_string("text".into()),
            cx
        ));
        assert!(!start(
            terminal.shared(),
            "unreachable.invalid",
            &ClipboardItem::new_string(String::new()),
            cx
        ));
        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, crate::app_icon::PNG.to_vec());
        assert!(start(
            terminal.shared(),
            "unreachable.invalid",
            &ClipboardItem::new_image(&image),
            cx
        ));
    });
}

#[gpui_kit::test]
#[cfg(unix)]
fn upload_errors_are_presented_over_the_requesting_terminal(cx: &mut gpui_kit::TestAppContext) {
    use super::super::surface::tests::{ECHO, Fixture};
    let f = Fixture::new(cx, ECHO, "");
    let key = f.shared().key();
    f.cx.cx.update(|cx| show_error(key, "ssh: upload refused", cx));
    f.cx.run_until_parked();
    f.cx.update(|window, cx| {
        let alert = crate::modal::front_alert(window.window_handle(), cx).expect("the upload error is visible");
        assert_eq!(alert.read(cx).spec().heading, "Failed to paste image");
        assert_eq!(alert.read(cx).spec().body, "ssh: upload refused");
    });
    f.cx.update(crate::modal::dismiss_alerts);
    assert!(
        !f.cx
            .update(|window, cx| crate::modal::alert_open(window.window_handle(), cx))
    );
}

#[test]
#[cfg(unix)]
fn a_filename_paste_reports_when_the_program_has_closed_its_pty() {
    use ngterm_term::{GridSize, Pane, PaneCommand, PaneConfig, PaneEvent};
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let mut command = PaneCommand::new("/bin/sh").args(["-c", "exit 0"]).cwd(dir.path());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            command = command.env_remove(key);
        }
    }
    let pane = Pane::spawn(
        &command,
        &PaneConfig::new(GridSize {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        }),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pane
        .take_events()
        .iter()
        .any(|event| matches!(event, PaneEvent::Exited(_)))
    {
        assert!(Instant::now() < deadline, "the test program did not exit");
        std::thread::yield_now();
    }
    let error = loop {
        if let Err(error) = paste_filename(&pane, "/tmp/ngterm-paste.0123456789/clipboard.png") {
            break error;
        }
        assert!(Instant::now() < deadline, "the exited program kept accepting input");
        std::thread::yield_now();
    };
    assert_eq!(error, "the pane's pty is closed");
}
