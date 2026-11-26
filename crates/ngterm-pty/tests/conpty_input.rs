//! What a Windows pseudoconsole hands its program of the control strings a terminal writes to its input. A Windows
//! viewer's remote pane runs ssh in one, so the attach bridge's envelopes reach the origin only in a form that passes
//! it (`ngterm_core::attach_bridge::APC_PREFIX`). The program is this test binary itself ([`input_probe`]), whose
//! console input is in virtual terminal mode, as ssh's is.
#![cfg(windows)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ngterm_pty::{Actor, ActorConfig, Command, ExitReason, Size, Spawned};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PATIENCE: Duration = Duration::from_secs(60);
const PROBE: &str = "NGTERM_PTY_INPUT_PROBE";
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const OSC_ENVELOPE: &[u8] = b"\x1b]777;ngterm-attach;e30=\x1b\\";
const APC_ENVELOPE: &[u8] = b"\x1b_777;ngterm-attach;e30=\x1b\\";

/// Not a test of its own: the pseudoconsole's program in the test below.
#[test]
#[ignore = "the input test's console program"]
fn input_probe() {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_INPUT, GetStdHandle, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW, STD_INPUT_HANDLE,
        SetConsoleMode,
    };
    if std::env::var_os(PROBE).is_none() {
        return;
    }
    // SAFETY: a plain call on the process's standard handle.
    let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    // SAFETY: the console input handle; ssh sets the same mode.
    unsafe { SetConsoleMode(input, ENABLE_VIRTUAL_TERMINAL_INPUT) };
    print!("PROBE-READY\r\n");
    let mut got: Vec<u16> = Vec::new();
    let mut records = [INPUT_RECORD::default(); 128];
    while !got.contains(&u16::from(b'.')) {
        let mut count = 0_u32;
        // SAFETY: the console input handle and a buffer of `records.len()` records.
        if unsafe { ReadConsoleInputW(input, records.as_mut_ptr(), 128, &raw mut count) } == 0 {
            break;
        }
        for record in records.iter().take(count as usize) {
            if u32::from(record.EventType) != KEY_EVENT {
                continue;
            }
            // SAFETY: a key event's record holds the key member.
            let key = unsafe { record.Event.KeyEvent };
            // SAFETY: the console fills the UTF-16 member for the W API.
            let unit = unsafe { key.uChar.UnicodeChar };
            if key.bKeyDown != 0 && unit != 0 {
                got.extend(std::iter::repeat_n(unit, usize::from(key.wRepeatCount.max(1))));
            }
        }
    }
    let got = String::from_utf16_lossy(&got).into_bytes();
    let form = |envelope: &[u8]| {
        if got.windows(envelope.len()).any(|window| window == envelope) {
            "passed"
        } else {
            "dropped"
        }
    };
    print!("osc={} apc={}\r\n", form(OSC_ENVELOPE), form(APC_ENVELOPE));
}

#[test]
fn the_pseudoconsole_drops_an_osc_from_its_input_and_passes_an_apc() -> TestResult {
    let exe = std::env::current_exe()?;
    let command = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| key.to_str().is_some_and(|key| key.starts_with("NGTERM_")))
        .fold(Command::new(exe), Command::env_remove)
        .args(["--ignored", "--exact", "input_probe", "--nocapture"])
        .env(PROBE, "1");
    let Spawned { master, mut child } = ngterm_pty::spawn(&command, Size::cells(24, 120))?;
    let output = Arc::new(Mutex::new(Vec::new()));
    let gathered = Arc::clone(&output);
    let mut answered = 0;
    let (exits, exit) = mpsc::channel();
    let mut config = ActorConfig::new(
        "conpty-input",
        master,
        Box::new(move |bytes: &[u8]| {
            let mut output = gathered.lock().unwrap_or_else(PoisonError::into_inner);
            output.extend_from_slice(bytes);
            let queries = output
                .windows(CURSOR_QUERY.len())
                .filter(|window| *window == CURSOR_QUERY)
                .count();
            let replies = (answered..queries).map(|_| b"\x1b[1;1R".to_vec()).collect();
            answered = queries;
            replies
        }),
    );
    config.on_exit = Some(Box::new(move |reason| {
        let _ = exits.send(reason);
    }));
    let handle = Actor::spawn(config)?;
    let text = || String::from_utf8_lossy(&output.lock().unwrap_or_else(PoisonError::into_inner)).into_owned();
    let expect = |needle: &str| {
        let deadline = Instant::now() + PATIENCE;
        while !text().contains(needle) {
            assert!(Instant::now() < deadline, "no {needle:?} in the output: {:?}", text());
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    expect("PROBE-READY");
    handle.write_input(&[OSC_ENVELOPE, APC_ENVELOPE, b"."].concat())?;
    let deadline = Instant::now() + PATIENCE;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            child.killer().kill_tree()?;
            return Err(format!("the probe never read the end of its input: {:?}", text()).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.child_exited();
    assert_eq!(exit.recv_timeout(PATIENCE)?, ExitReason::Eof);
    assert!(text().contains("osc=dropped apc=passed"), "{:?}", text());
    Ok(())
}
