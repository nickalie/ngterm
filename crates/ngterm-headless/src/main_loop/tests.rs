use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::*;

type Log = Vec<&'static str>;

fn push(name: &'static str) -> MainTask<Log> {
    Box::new(move |log: &mut Log| log.push(name))
}

fn close_after(main_loop: &Arc<ThreadMainLoop<Log>>) -> MainTask<Log> {
    let closing = Arc::clone(main_loop);
    Box::new(move |_: &mut Log| closing.close())
}

#[test]
fn tasks_run_in_posting_order_and_the_after_hook_follows_each() {
    let main_loop = Arc::new(ThreadMainLoop::<Log>::default());
    main_loop.post(push("a")).unwrap();
    main_loop.post(push("b")).unwrap();
    main_loop.post(close_after(&main_loop)).unwrap();
    let mut log = Log::new();
    let mut afters = 0;
    main_loop.run(&mut log, |_| afters += 1);
    assert_eq!(log, ["a", "b"]);
    assert_eq!(afters, 3);
}

#[test]
fn delayed_tasks_run_by_their_deadlines_after_the_ready_ones() {
    let main_loop = Arc::new(ThreadMainLoop::<Log>::default());
    let started = Instant::now();
    main_loop.post_after(Duration::from_millis(60), push("late")).unwrap();
    main_loop.post_after(Duration::from_millis(20), push("early")).unwrap();
    main_loop.post(push("now")).unwrap();
    main_loop
        .post_after(Duration::from_millis(80), close_after(&main_loop))
        .unwrap();
    let mut log = Log::new();
    main_loop.run(&mut log, |_| {});
    assert_eq!(log, ["now", "early", "late"]);
    assert!(started.elapsed() >= Duration::from_millis(80));
}

#[test]
fn equal_deadlines_keep_their_posting_order() {
    let main_loop = Arc::new(ThreadMainLoop::<Log>::default());
    for name in ["1", "2", "3"] {
        main_loop.post_after(Duration::ZERO, push(name)).unwrap();
    }
    main_loop.post_after(Duration::ZERO, close_after(&main_loop)).unwrap();
    let mut log = Log::new();
    main_loop.run(&mut log, |_| {});
    assert_eq!(log, ["1", "2", "3"]);
}

#[test]
fn another_thread_wakes_a_waiting_loop() {
    let main_loop = Arc::new(ThreadMainLoop::<Log>::default());
    let posting = Arc::clone(&main_loop);
    let poster = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        posting.post(push("from a thread")).unwrap();
        posting.post(close_after(&posting)).unwrap();
    });
    let mut log = Log::new();
    main_loop.run(&mut log, |_| {});
    poster.join().unwrap();
    assert_eq!(log, ["from a thread"]);
}

#[test]
fn a_closed_loop_refuses_work_and_drops_what_waited() {
    let main_loop = Arc::new(ThreadMainLoop::<Log>::default());
    main_loop.post(close_after(&main_loop)).unwrap();
    main_loop.post(push("dropped")).unwrap();
    main_loop
        .post_after(Duration::from_millis(1), push("dropped too"))
        .unwrap();
    let mut log = Log::new();
    main_loop.run(&mut log, |_| {});
    assert!(log.is_empty());
    assert_eq!(main_loop.post(push("refused")), Err(MainLoopClosed));
    assert_eq!(
        main_loop.post_after(Duration::ZERO, push("refused")),
        Err(MainLoopClosed)
    );
}
