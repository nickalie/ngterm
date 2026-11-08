//! The outer terminal on Windows: the console in VT mode both ways (herdr's `enable_windows_virtual_terminal_input`,
//! its input then read as records, `windows_console_input_items`), the window's size from the screen buffer, and the
//! input's reader, which also reports the console's window-size events, and a watcher polling the window's size.
//!
//! herdr: `src/client/terminal_setup.rs` and `src/client/input/windows_vti.rs` at `HERDR_REV` (`build/pins.env`),
//! licensed under the Apache License 2.0, modified as the parent module's docs say.

use std::fs::File;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::mem::ManuallyDrop;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_EXTENDED_FLAGS,
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
    GetConsoleCP, GetConsoleMode, GetConsoleOutputCP, GetConsoleScreenBufferInfo, GetStdHandle, INPUT_RECORD,
    KEY_EVENT, KEY_EVENT_RECORD, LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED,
    ReadConsoleInputW, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleMode, SetConsoleOutputCP,
    WINDOW_BUFFER_SIZE_EVENT,
};

use super::{TtyEvent, TtySize};
use crate::protocol::GridSize;

/// UTF-8's code page.
const CP_UTF8: u32 = 65001;

/// The size of one read of a piped input.
const READ_SIZE: usize = 16 * 1024;

/// The console records one read takes.
const RECORDS: u32 = 64;

/// How often the window's size is looked at.
const SIZE_POLL: Duration = Duration::from_millis(200);

/// The space bar's virtual-key code (`VK_SPACE`).
const VK_SPACE: u16 = 0x20;
/// The 2 key's virtual-key code (`'2'`), Ctrl+@ with Shift.
const VK_2: u16 = 0x32;

pub(super) fn is_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn std_handle(which: u32) -> Option<HANDLE> {
    // SAFETY: querying a standard handle has no preconditions.
    let handle = unsafe { GetStdHandle(which) };
    (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(handle)
}

fn console_mode(handle: HANDLE) -> io::Result<CONSOLE_MODE> {
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: a console handle and a valid out-pointer.
    if unsafe { GetConsoleMode(handle, &raw mut mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(mode)
}

fn set_console_mode(handle: HANDLE, mode: CONSOLE_MODE) -> io::Result<()> {
    // SAFETY: a console handle.
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The console in VT mode: its input as VT sequences with window-size events and no line editing, echo or Ctrl+C
/// handling (`ENABLE_EXTENDED_FLAGS` without quick edit, so the mouse reaches the session), its output's VT sequences
/// processed and no automatic return; both in UTF-8. Dropped, everything is restored.
#[derive(Debug)]
pub(super) struct RawMode {
    input: HANDLE,
    output: HANDLE,
    input_mode: CONSOLE_MODE,
    output_mode: CONSOLE_MODE,
    input_code_page: u32,
    output_code_page: u32,
}

impl RawMode {
    pub(super) fn enter() -> io::Result<Self> {
        let not_console = || io::Error::other("the standard handles are not a console");
        let input = std_handle(STD_INPUT_HANDLE).ok_or_else(not_console)?;
        let output = std_handle(STD_OUTPUT_HANDLE).ok_or_else(not_console)?;
        let input_mode = console_mode(input)?;
        let output_mode = console_mode(output)?;
        // SAFETY: no preconditions.
        let input_code_page = unsafe { GetConsoleCP() };
        // SAFETY: no preconditions.
        let output_code_page = unsafe { GetConsoleOutputCP() };
        let raw = Self {
            input,
            output,
            input_mode,
            output_mode,
            input_code_page,
            output_code_page,
        };
        set_console_mode(
            input,
            ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS,
        )?;
        set_console_mode(
            output,
            output_mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN,
        )?;
        // SAFETY: no preconditions; a failure leaves the code page as it was.
        unsafe { SetConsoleCP(CP_UTF8) };
        // SAFETY: as above.
        unsafe { SetConsoleOutputCP(CP_UTF8) };
        Ok(raw)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = set_console_mode(self.input, self.input_mode);
        let _ = set_console_mode(self.output, self.output_mode);
        // SAFETY: no preconditions.
        unsafe { SetConsoleCP(self.input_code_page) };
        // SAFETY: no preconditions.
        unsafe { SetConsoleOutputCP(self.output_code_page) };
    }
}

pub(super) fn size() -> Option<TtySize> {
    let output = std_handle(STD_OUTPUT_HANDLE)?;
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: a console output handle and a valid out-pointer.
    if unsafe { GetConsoleScreenBufferInfo(output, &raw mut info) } == 0 {
        return None;
    }
    let window = info.srWindow;
    let cols = u16::try_from(i32::from(window.Right) - i32::from(window.Left) + 1).ok()?;
    let rows = u16::try_from(i32::from(window.Bottom) - i32::from(window.Top) + 1).ok()?;
    Some(TtySize {
        grid: GridSize { cols, rows },
        pixels: None,
    })
}

pub(super) fn start_threads<E: From<TtyEvent> + Send + 'static>(
    raw: Option<&RawMode>,
    events: &Sender<E>,
) -> io::Result<()> {
    let input = events.clone();
    let console = raw.map(|raw| ConsoleInput(raw.input));
    let watch = console.is_some();
    thread::Builder::new()
        .name("attach-input".to_owned())
        .spawn(move || match console {
            Some(console) => read_console(&console, &input),
            None => read_piped(&input),
        })?;
    if watch {
        let resize = events.clone();
        thread::Builder::new()
            .name("attach-resize".to_owned())
            .spawn(move || watch_size(&resize))?;
    }
    Ok(())
}

/// Reports a change of the console's window size, polled: a console reports a change of its buffer's size as an input
/// record, but not every change of its window's (herdr relies on polling on Windows too).
fn watch_size<E: From<TtyEvent>>(events: &Sender<E>) {
    let mut last = size();
    loop {
        thread::sleep(SIZE_POLL);
        let now = size();
        if now != last {
            last = now;
            if events.send(TtyEvent::Resize.into()).is_err() {
                return;
            }
        }
    }
}

/// The console's input handle, moved to the reader.
struct ConsoleInput(HANDLE);

// SAFETY: a process-wide standard handle, valid from any thread.
unsafe impl Send for ConsoleInput {}

/// The console's input records until it fails: the characters of key presses (the VT bytes the console encodes the
/// keys, and the terminal's answers, as; NUL from the key a record without a character names, [`nul_key`]), and
/// window-size events.
fn read_console<E: From<TtyEvent>>(console: &ConsoleInput, events: &Sender<E>) {
    let mut records = [INPUT_RECORD::default(); RECORDS as usize];
    let mut decoder = Utf16Decoder::default();
    loop {
        let mut read = 0_u32;
        // SAFETY: a console input handle, a buffer of `RECORDS` records and a valid out-pointer.
        let ok = unsafe { ReadConsoleInputW(console.0, records.as_mut_ptr(), RECORDS, &raw mut read) };
        if ok == 0 {
            break;
        }
        let mut bytes = Vec::new();
        let mut resized = false;
        for record in records.iter().take(read as usize) {
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    // SAFETY: the record is a key event, so the union holds a key event record.
                    let key = unsafe { record.Event.KeyEvent };
                    key_input(&key, &mut decoder, &mut bytes);
                }
                WINDOW_BUFFER_SIZE_EVENT => resized = true,
                _ => {}
            }
        }
        if !bytes.is_empty() && events.send(TtyEvent::Input(bytes).into()).is_err() {
            return;
        }
        if resized && events.send(TtyEvent::Resize.into()).is_err() {
            return;
        }
    }
    let _ = events.send(TtyEvent::InputEnd.into());
}

/// The bytes a key event record carries: a press's character, as many times as it repeats (NUL from the key when it
/// carries none, [`nul_key`]); nothing for a release.
fn key_input(key: &KEY_EVENT_RECORD, decoder: &mut Utf16Decoder, out: &mut Vec<u8>) {
    if key.bKeyDown == 0 {
        return;
    }
    // SAFETY: both members of the character union are plain data; the W API fills `UnicodeChar`.
    let unit = unsafe { key.uChar.UnicodeChar };
    for _ in 0..key.wRepeatCount.max(1) {
        if unit == 0 {
            out.extend_from_slice(nul_key(key.wVirtualKeyCode, key.dwControlKeyState));
        } else {
            decoder.push(unit, out);
        }
    }
}

/// The bytes of a key press whose record carries no character: NUL for Ctrl+Space, Ctrl+2 and Ctrl+@ (Ctrl+Shift+2),
/// as xterm sends them, ESC first with the left Alt; nothing for any other (a modifier alone, a key the console encodes
/// as nothing, a chord with the right Alt, which is `AltGr` with Ctrl on the layouts that have it).
fn nul_key(virtual_key: u16, control_state: u32) -> &'static [u8] {
    let ctrl = control_state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
    let alt = control_state & LEFT_ALT_PRESSED != 0;
    let alt_gr = control_state & RIGHT_ALT_PRESSED != 0;
    match (virtual_key, ctrl && !alt_gr, alt) {
        (VK_SPACE | VK_2, true, false) => b"\0",
        (VK_SPACE | VK_2, true, true) => b"\x1b\0",
        _ => &[],
    }
}

/// UTF-16 code units to UTF-8, a surrogate pair split across records joined.
#[derive(Debug, Default)]
struct Utf16Decoder {
    high: Option<u16>,
}

impl Utf16Decoder {
    fn push(&mut self, unit: u16, out: &mut Vec<u8>) {
        if unit == 0 {
            return;
        }
        let units: Vec<u16> = match (self.high.take(), unit) {
            (None, 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                return;
            }
            (Some(high), 0xDC00..=0xDFFF) => vec![high, unit],
            (_, unit) => vec![unit],
        };
        for decoded in char::decode_utf16(units) {
            let ch = decoded.unwrap_or(char::REPLACEMENT_CHARACTER);
            let mut buf = [0_u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
    }
}

/// A piped standard input's bytes as they come, until its end.
fn read_piped<E: From<TtyEvent>>(events: &Sender<E>) {
    let mut stdin = io::stdin().lock();
    let mut buf = vec![0_u8; READ_SIZE];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(read) => {
                if events.send(TtyEvent::Input(buf[..read].to_vec()).into()).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = events.send(TtyEvent::InputEnd.into());
}

pub(super) fn write_out(bytes: &[u8]) -> io::Result<()> {
    let stdout = io::stdout();
    let handle = stdout.as_raw_handle();
    // SAFETY: the process's standard output handle, borrowed for the write and never closed (`ManuallyDrop`).
    let mut file = ManuallyDrop::new(unsafe { File::from_raw_handle(handle) });
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Output, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    use windows_sys::Win32::System::Console::{
        FOCUS_EVENT, FOCUS_EVENT_RECORD, KEY_EVENT_RECORD_0, SHIFT_PRESSED, SMALL_RECT, STD_HANDLE,
        SetConsoleWindowInfo, SetStdHandle, WriteConsoleInputW,
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    use super::*;

    #[test]
    fn a_record_without_a_character_is_nul_only_for_the_keys_that_send_it() {
        assert_eq!(nul_key(VK_SPACE, LEFT_CTRL_PRESSED), b"\0");
        assert_eq!(nul_key(VK_2, RIGHT_CTRL_PRESSED | SHIFT_PRESSED), b"\0", "Ctrl+@");
        assert_eq!(nul_key(VK_SPACE, LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED), b"\x1b\0");
        // modifiers alone, and keys the console encodes as nothing
        assert_eq!(nul_key(0x11, LEFT_CTRL_PRESSED), b"");
        assert_eq!(nul_key(0x10, SHIFT_PRESSED), b"");
        assert_eq!(nul_key(VK_SPACE, 0), b"");
        assert_eq!(nul_key(VK_2, LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED), b"", "AltGr+2");
        assert_eq!(nul_key(0x7C, LEFT_CTRL_PRESSED), b"", "F13");
    }

    #[test]
    fn utf16_units_decode_to_utf8_with_split_surrogates() {
        let mut decoder = Utf16Decoder::default();
        let mut out = Vec::new();
        for unit in "a€".encode_utf16() {
            decoder.push(unit, &mut out);
        }
        let pair: Vec<u16> = "😀".encode_utf16().collect();
        decoder.push(pair[0], &mut out);
        decoder.push(pair[1], &mut out);
        decoder.push(0x1b, &mut out);
        assert_eq!(out, "a€😀\x1b".as_bytes());
    }

    #[test]
    fn a_nul_unit_decodes_to_nothing() {
        let mut out = Vec::new();
        Utf16Decoder::default().push(0, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_handle_that_is_no_console_has_no_mode_to_read_or_set() {
        let file = tempfile::tempfile().unwrap();
        let handle = file.as_raw_handle();
        assert_eq!(
            console_mode(handle).unwrap_err().raw_os_error(),
            Some(6),
            "ERROR_INVALID_HANDLE"
        );
        assert_eq!(
            set_console_mode(handle, ENABLE_WINDOW_INPUT)
                .unwrap_err()
                .raw_os_error(),
            Some(6)
        );
    }

    #[test]
    fn console_input_that_cannot_be_read_ends_at_once() {
        let file = tempfile::tempfile().unwrap();
        let (send, receive) = mpsc::channel::<TtyEvent>();
        read_console(&ConsoleInput(file.as_raw_handle()), &send);
        drop(send);
        assert_eq!(receive.iter().collect::<Vec<_>>(), [TtyEvent::InputEnd]);
    }

    /// Set in the child a test starts (this test binary again, running only that test): which test's child it is.
    const CHILD: &str = "ATTACH_TTY_TEST_CHILD";
    /// What a child writes to its standard error once its checks passed.
    const CHILD_DONE: &str = "attach-tty child done";

    /// Whether this process is the child of test `name`.
    fn is_child(name: &str) -> bool {
        std::env::var_os(CHILD).is_some_and(|child| child == name)
    }

    /// This test binary run again as the child of test `name` (running that test alone) on `stdin`, with `input`
    /// written to it when it is piped, in a console of its own without a window; it passed once it reported
    /// [`CHILD_DONE`]. Ended after a minute.
    fn run_child(name: &str, stdin: Stdio, input: Option<&[u8]>) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("attach_tty::windows::tests::{name}"), "--nocapture"])
            .env(CHILD, name)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        // held open until the child ends: its input ends only when the child stops reading
        let mut writer = child.stdin.take();
        if let (Some(writer), Some(input)) = (writer.as_mut(), input) {
            writer.write_all(input).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let Output { status, stderr, .. } = child.wait_with_output().unwrap();
        drop(writer);
        let stderr = String::from_utf8_lossy(&stderr);
        assert!(status.success() && stderr.contains(CHILD_DONE), "{status}: {stderr}");
    }

    /// The child's console: its input and its output buffer.
    fn console() -> (File, File) {
        let open = |name| OpenOptions::new().read(true).write(true).open(name).unwrap();
        (open("CONIN$"), open("CONOUT$"))
    }

    fn set_std(which: STD_HANDLE, handle: &File) {
        // SAFETY: an open handle this process keeps for the rest of the child's run.
        assert_ne!(unsafe { SetStdHandle(which, handle.as_raw_handle()) }, 0);
    }

    fn key(unit: u16, down: bool) -> INPUT_RECORD {
        let mut record = INPUT_RECORD {
            EventType: u16::try_from(KEY_EVENT).unwrap(),
            ..INPUT_RECORD::default()
        };
        record.Event.KeyEvent = KEY_EVENT_RECORD {
            bKeyDown: down.into(),
            wRepeatCount: 1,
            uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
            ..KEY_EVENT_RECORD::default()
        };
        record
    }

    fn write_input(console: &File, records: &[INPUT_RECORD]) {
        let mut written = 0_u32;
        let length = u32::try_from(records.len()).unwrap();
        // SAFETY: a console input handle, `length` records and a valid out-pointer.
        let ok = unsafe { WriteConsoleInputW(console.as_raw_handle(), records.as_ptr(), length, &raw mut written) };
        assert_ne!(ok, 0, "{}", io::Error::last_os_error());
        assert_eq!(written, length);
    }

    /// The console in raw mode reads key presses as their characters and a window-size record as a size change,
    /// ignores the records it has no use for (focus), stops once nobody takes its events, and is restored on drop.
    #[test]
    fn a_console_reads_keys_and_size_changes_until_nobody_listens() {
        if !is_child("a_console_reads_keys_and_size_changes_until_nobody_listens") {
            run_child(
                "a_console_reads_keys_and_size_changes_until_nobody_listens",
                Stdio::null(),
                None,
            );
            return;
        }
        let (input, output) = console();
        set_std(STD_INPUT_HANDLE, &input);
        set_std(STD_OUTPUT_HANDLE, &output);
        let before = console_mode(input.as_raw_handle()).unwrap();
        let raw = RawMode::enter().unwrap();
        assert_eq!(
            console_mode(input.as_raw_handle()).unwrap(),
            ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS
        );
        let mut focus = INPUT_RECORD {
            EventType: u16::try_from(FOCUS_EVENT).unwrap(),
            ..INPUT_RECORD::default()
        };
        focus.Event.FocusEvent = FOCUS_EVENT_RECORD { bSetFocus: 1 };
        // its size is not read: the size is the window's
        let resize = INPUT_RECORD {
            EventType: u16::try_from(WINDOW_BUFFER_SIZE_EVENT).unwrap(),
            ..INPUT_RECORD::default()
        };
        write_input(
            &input,
            &[focus, key(u16::from(b'a'), true), key(u16::from(b'a'), false), resize],
        );
        let (send, receive) = mpsc::channel::<TtyEvent>();
        let console_input = ConsoleInput(input.as_raw_handle());
        let reader = thread::spawn(move || read_console(&console_input, &send));
        let timeout = Duration::from_secs(10);
        assert_eq!(receive.recv_timeout(timeout), Ok(TtyEvent::Input(b"a".to_vec())));
        assert_eq!(receive.recv_timeout(timeout), Ok(TtyEvent::Resize));
        drop(receive);
        write_input(&input, &[key(u16::from(b'b'), true)]);
        reader.join().unwrap();
        drop(raw);
        assert_eq!(console_mode(input.as_raw_handle()).unwrap(), before);
        eprintln!("{CHILD_DONE}");
    }

    /// A console that refuses a mode fails raw mode, and leaves both handles as they were.
    #[test]
    fn a_console_refusing_raw_mode_is_left_as_it_was() {
        if !is_child("a_console_refusing_raw_mode_is_left_as_it_was") {
            run_child("a_console_refusing_raw_mode_is_left_as_it_was", Stdio::null(), None);
            return;
        }
        let (input, output) = console();
        // an output buffer for the input: it takes no input mode
        set_std(STD_INPUT_HANDLE, &output);
        set_std(STD_OUTPUT_HANDLE, &output);
        let before = console_mode(output.as_raw_handle()).unwrap();
        assert!(RawMode::enter().unwrap_err().raw_os_error().is_some());
        assert_eq!(console_mode(output.as_raw_handle()).unwrap(), before);
        // the input for the output too, which then would echo without line input: the console refuses that
        let second = input.try_clone().unwrap();
        set_console_mode(input.as_raw_handle(), ENABLE_EXTENDED_FLAGS).unwrap();
        set_std(STD_INPUT_HANDLE, &input);
        set_std(STD_OUTPUT_HANDLE, &second);
        assert!(RawMode::enter().unwrap_err().raw_os_error().is_some());
        assert_eq!(console_mode(input.as_raw_handle()).unwrap(), ENABLE_EXTENDED_FLAGS);
        eprintln!("{CHILD_DONE}");
    }

    /// The window's width set to `cols`, its height kept.
    fn set_window_width(output: &File, cols: i16) {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: a console output handle and a valid out-pointer.
        let read = unsafe { GetConsoleScreenBufferInfo(output.as_raw_handle(), &raw mut info) };
        assert_ne!(read, 0);
        let window = SMALL_RECT {
            Left: 0,
            Top: info.srWindow.Top,
            Right: cols - 1,
            Bottom: info.srWindow.Bottom,
        };
        // SAFETY: a console output handle and a rectangle that lives across the call.
        let ok = unsafe { SetConsoleWindowInfo(output.as_raw_handle(), 1, &raw const window) };
        assert_ne!(ok, 0, "{}", io::Error::last_os_error());
        assert_eq!(size().map(|size| size.grid.cols), u16::try_from(cols).ok());
    }

    /// The watcher reports a change of the window's size, and stops at the first change once nobody listens.
    #[test]
    fn the_size_watcher_reports_changes_until_nobody_listens() {
        if !is_child("the_size_watcher_reports_changes_until_nobody_listens") {
            run_child(
                "the_size_watcher_reports_changes_until_nobody_listens",
                Stdio::null(),
                None,
            );
            return;
        }
        let (_input, output) = console();
        set_std(STD_OUTPUT_HANDLE, &output);
        let (send, receive) = mpsc::channel::<TtyEvent>();
        let (ended, watcher_ended) = mpsc::channel();
        thread::spawn(move || {
            watch_size(&send);
            let _ = ended.send(());
        });
        // until the watcher has taken the first size and seen it change
        let mut widths = [40, 30].into_iter().cycle();
        let reported = (0..20).any(|_| {
            set_window_width(&output, widths.next().unwrap());
            receive.recv_timeout(SIZE_POLL * 2) == Ok(TtyEvent::Resize)
        });
        assert!(reported, "no size change was reported");
        drop(receive);
        let stopped = (0..20).any(|_| {
            set_window_width(&output, widths.next().unwrap());
            watcher_ended.recv_timeout(SIZE_POLL * 2).is_ok()
        });
        assert!(stopped, "the watcher outlived its listener");
        eprintln!("{CHILD_DONE}");
    }

    /// A piped input's reader stops at its first read once nobody takes its events, though the input stays open.
    #[test]
    fn a_piped_input_with_nobody_listening_ends_its_reader() {
        if !is_child("a_piped_input_with_nobody_listening_ends_its_reader") {
            run_child(
                "a_piped_input_with_nobody_listening_ends_its_reader",
                Stdio::piped(),
                Some(b"typed"),
            );
            return;
        }
        let (send, receive) = mpsc::channel::<TtyEvent>();
        drop(receive);
        read_piped(&send);
        eprintln!("{CHILD_DONE}");
    }

    /// An input that fails to read (a file open for writing only) ends at once.
    #[test]
    fn an_input_that_fails_to_read_ends_at_once() {
        if !is_child("an_input_that_fails_to_read_ends_at_once") {
            let directory = tempfile::tempdir().unwrap();
            let write_only = File::create(directory.path().join("input")).unwrap();
            run_child(
                "an_input_that_fails_to_read_ends_at_once",
                Stdio::from(write_only),
                None,
            );
            return;
        }
        let (send, receive) = mpsc::channel::<TtyEvent>();
        read_piped(&send);
        drop(send);
        assert_eq!(receive.iter().collect::<Vec<_>>(), [TtyEvent::InputEnd]);
        eprintln!("{CHILD_DONE}");
    }
}
