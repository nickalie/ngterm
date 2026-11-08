//! Where the terminal's replies to the program leave the pane: the PTY actor's outbox, a live pane's session, or a
//! stream pane's outbound queue ([`crate::stream`]).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[cfg(feature = "local")]
use ngterm_pty::ActorHandle;

#[cfg(feature = "local")]
use crate::session_io::{self, Live};
use crate::stream::Outbound;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where the terminal's replies to the program go: the PTY actor's outbox, in the order they are made (never
/// refused; the actor drops them past its hard cap, `ngterm_pty::REPLY_CAP`). Replies made before the actor runs
/// (a terminal driven without a PTY) wait here, and none are taken once the pane closed it. A live pane's terminal
/// answers the program nothing but Kitty graphics commands, the server answering the rest
/// (`crate::session_io`): those answers are kept for the server's next barrier. A stream pane's replies join its
/// outbound queue, in order with the program's input.
///
/// A reply is queued with `ActorHandle::write_reply`, which waits for a read callback in progress. That is safe
/// under the VT lock because the pane's read callback only copies into the pane's mailbox ([`crate::mailbox`]) and
/// never takes the VT lock.
#[derive(Debug, Default)]
pub(crate) struct ReplyOutlet {
    state: Mutex<Outlet>,
}

#[derive(Debug)]
enum Outlet {
    /// No actor yet: the replies so far, and their bytes.
    Buffered(Vec<Vec<u8>>, usize),
    #[cfg(feature = "local")]
    Actor(ActorHandle),
    /// A live pane's: its Kitty graphics answers since the last barrier, and the attach the pane's own input goes to.
    #[cfg(feature = "local")]
    Live(Vec<u8>, Arc<Live>),
    /// A stream pane's outbound queue.
    Stream(Arc<Outbound>),
    Closed,
}

impl Default for Outlet {
    fn default() -> Self {
        Self::Buffered(Vec::new(), 0)
    }
}

impl ReplyOutlet {
    /// Queue a reply for the program.
    pub(crate) fn push_reply(&self, data: &[u8]) {
        let mut state = lock(&self.state);
        match &mut *state {
            Outlet::Buffered(replies, bytes) => {
                replies.push(data.to_vec());
                *bytes += data.len();
            }
            #[cfg(feature = "local")]
            Outlet::Actor(actor) => {
                let actor = actor.clone();
                drop(state);
                actor.write_reply(data);
            }
            #[cfg(feature = "local")]
            Outlet::Live(answers, _) => session_io::keep_kitty_answer(answers, data),
            Outlet::Stream(outbound) => outbound.push_reply(data),
            Outlet::Closed => {}
        }
    }

    /// Queue bytes the pane itself sends the program (a clear's form feed): a reply on a PTY or a stream, a live
    /// pane's input.
    pub(crate) fn push_input(&self, data: &[u8]) {
        #[cfg(feature = "local")]
        {
            let state = lock(&self.state);
            if let Outlet::Live(_, live) = &*state {
                let live = Arc::clone(live);
                drop(state);
                live.program_input(data);
                return;
            }
        }
        self.push_reply(data);
    }

    /// Answer as a live pane's terminal from now on (before it parses anything).
    #[cfg(feature = "local")]
    pub(crate) fn go_live(&self, live: Arc<Live>) {
        *lock(&self.state) = Outlet::Live(Vec::new(), live);
    }

    /// A live pane's Kitty graphics answers since the last call.
    #[cfg(feature = "local")]
    pub(crate) fn take_kitty_answers(&self) -> Vec<u8> {
        match &mut *lock(&self.state) {
            Outlet::Live(answers, _) => std::mem::take(answers),
            _ => Vec::new(),
        }
    }

    /// Send replies to `actor` from now on, the ones made so far first.
    #[cfg(feature = "local")]
    pub(crate) fn attach(&self, actor: &ActorHandle) {
        let mut state = lock(&self.state);
        if matches!(*state, Outlet::Closed) {
            return;
        }
        if let Outlet::Buffered(replies, _) = std::mem::replace(&mut *state, Outlet::Actor(actor.clone())) {
            for reply in &replies {
                actor.write_reply(reply);
            }
        }
    }

    /// Send replies to a stream pane's `outbound` queue from now on, the ones made so far first.
    pub(crate) fn attach_stream(&self, outbound: &Arc<Outbound>) {
        let mut state = lock(&self.state);
        if matches!(*state, Outlet::Closed) {
            return;
        }
        if let Outlet::Buffered(replies, _) = std::mem::replace(&mut *state, Outlet::Stream(Arc::clone(outbound))) {
            for reply in &replies {
                outbound.push_reply(reply);
            }
        }
    }

    /// Take no more replies, and let go of the actor.
    pub(crate) fn close(&self) {
        *lock(&self.state) = Outlet::Closed;
    }

    /// The reply bytes waiting for the actor, or queued in it with the program's input.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        let state = lock(&self.state);
        match &*state {
            Outlet::Buffered(_, bytes) => *bytes,
            #[cfg(feature = "local")]
            Outlet::Actor(actor) => actor.queued(),
            #[cfg(feature = "local")]
            Outlet::Live(answers, _) => answers.len(),
            Outlet::Stream(outbound) => outbound.queued(),
            Outlet::Closed => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_wait_for_the_actor_and_none_are_taken_once_closed() {
        let outlet = ReplyOutlet::default();
        assert_eq!(outlet.queued(), 0);
        outlet.push_reply(b"\x1b[0n");
        outlet.push_reply(b"\x1b[1;1R");
        assert_eq!(outlet.queued(), 10);
        outlet.close();
        outlet.push_reply(b"late");
        assert_eq!(outlet.queued(), 0);
    }

    #[test]
    fn a_stream_takes_the_replies_made_before_it_first() {
        let outlet = ReplyOutlet::default();
        outlet.push_reply(b"\x1b[0n");
        let outbound = Arc::new(Outbound::new(16));
        outlet.attach_stream(&outbound);
        outlet.push_input(b"\x0c");
        assert_eq!(outlet.queued(), 5);
        assert_eq!(
            outbound.next(None),
            Ok(Some(crate::Outgoing::Bytes(b"\x1b[0n\x0c".to_vec())))
        );
    }

    #[test]
    fn a_new_stream_takes_the_replies_from_then_on() {
        let outlet = ReplyOutlet::default();
        let (first, second) = (Arc::new(Outbound::new(16)), Arc::new(Outbound::new(16)));
        outlet.attach_stream(&first);
        outlet.push_reply(b"one");
        outlet.attach_stream(&second);
        outlet.push_reply(b"two");
        assert_eq!(first.next(None), Ok(Some(crate::Outgoing::Bytes(b"one".to_vec()))));
        assert_eq!(second.next(None), Ok(Some(crate::Outgoing::Bytes(b"two".to_vec()))));
    }

    #[test]
    fn a_closed_outlet_takes_no_stream_and_has_no_kitty_answers() {
        let outlet = ReplyOutlet::default();
        outlet.push_reply(b"\x1b_Gi=1;OK\x1b\\");
        // only a live pane's outlet keeps Kitty answers for a barrier
        #[cfg(feature = "local")]
        assert!(outlet.take_kitty_answers().is_empty());
        outlet.close();
        let outbound = Arc::new(Outbound::new(16));
        outlet.attach_stream(&outbound);
        outlet.push_reply(b"\x1b[0n");
        outlet.push_input(b"\x0c");
        assert_eq!(outbound.queued(), 0);
        assert_eq!(outlet.queued(), 0);
    }

    // the program is a POSIX shell script
    #[cfg(all(feature = "local", unix))]
    mod actor {
        use std::time::{Duration, Instant};

        use ngterm_pty::{Actor, ActorConfig, Command, Size, Spawned};

        use super::*;

        const SIZE: Size = Size {
            rows: 4,
            cols: 40,
            cell_width: 8,
            cell_height: 16,
        };

        /// A PTY running `script`, its actor collecting the output.
        fn actor(script: &str) -> (ActorHandle, ngterm_pty::Child, Arc<Mutex<Vec<u8>>>) {
            let Spawned { master, child } =
                ngterm_pty::spawn(&Command::new("/bin/sh").arg("-c").arg(script), SIZE).unwrap();
            let output = Arc::new(Mutex::new(Vec::new()));
            let read = Arc::clone(&output);
            let config = ActorConfig::new(
                "outlet-test",
                master,
                Box::new(move |bytes| {
                    lock(&read).extend_from_slice(bytes);
                    Vec::new()
                }),
            );
            (Actor::spawn(config).unwrap(), child, output)
        }

        fn wait_output(output: &Mutex<Vec<u8>>, needle: &str) {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let text = String::from_utf8_lossy(&lock(output)).into_owned();
                if text.contains(needle) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out: {text:?}");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        #[test]
        fn the_replies_made_before_the_actor_reach_the_program_first() {
            let (handle, mut child, output) = actor("stty -echo; read first; read second; echo \"got $first,$second\"");
            let outlet = ReplyOutlet::default();
            outlet.push_reply(b"one\n");
            outlet.attach(&handle);
            outlet.push_input(b"two\n");
            wait_output(&output, "got one,two");
            outlet.close();
            handle.shutdown();
            child.wait().unwrap();
        }

        #[test]
        fn a_closed_outlet_does_not_take_the_actor() {
            let (handle, mut child, _output) = actor("exec sleep 30");
            let outlet = ReplyOutlet::default();
            outlet.push_reply(b"early");
            outlet.close();
            outlet.attach(&handle);
            outlet.push_reply(b"late");
            assert!(matches!(*lock(&outlet.state), Outlet::Closed));
            assert_eq!(outlet.queued(), 0);
            child.killer().kill_tree().unwrap();
            handle.shutdown();
            child.wait().unwrap();
        }
    }
}
