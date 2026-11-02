//! The PTY side of a pane over `ngterm-pty` (openpty on Unix, `ConPTY` on Windows, herdr's PTY actor moving the
//! bytes): what a pane runs, its grid as the PTY's size, and what the pane reads of the PTY itself (its foreground
//! process group and its input modes). The outlet the terminal's replies leave through is [`crate::outlet`]'s.

use ngterm_pty::WriteError;

use crate::{GridSize, PaneError};

/// What a pane runs, and how: `ngterm-pty`'s command (its program, arguments, working directory and environment).
pub use ngterm_pty::Command as PaneCommand;

impl GridSize {
    pub(crate) fn pty(self) -> ngterm_pty::Size {
        ngterm_pty::Size {
            rows: self.rows,
            cols: self.cols,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl From<WriteError> for PaneError {
    fn from(err: WriteError) -> Self {
        match err {
            WriteError::Backpressure => Self::Backpressure,
            WriteError::Closed => Self::Closed,
        }
    }
}

/// What the pane reads of its PTY on its own thread, without asking the actor (which would wait on its thread): a
/// close-on-exec duplicate of the master on Unix, for `tcgetpgrp` and `tcgetattr`. Windows' pseudoconsole has
/// neither: the parent holds only its pipes and its handle, not the console's input buffer whose mode says whether
/// the program echoes, and no process group is the console's foreground. There the pane reads the child's job
/// instead, the processes of its console, for a stand-in foreground ([`Tty::foreground_pid`]).
#[derive(Debug)]
pub(crate) struct Tty {
    #[cfg(unix)]
    master: std::os::fd::OwnedFd,
    /// The pane's program and the job its tree runs in, never held open from here.
    #[cfg(windows)]
    child: (u32, ngterm_pty::JobView),
}

impl Tty {
    /// Keep what the pane reads of `master` before the actor takes it.
    #[cfg(unix)]
    pub(crate) fn of(master: &ngterm_pty::Master, _child: &ngterm_pty::Child) -> Result<Self, PaneError> {
        use std::os::fd::AsFd;

        let master = master
            .as_fd()
            .try_clone_to_owned()
            .map_err(|err| PaneError::Pty(err.to_string()))?;
        Ok(Self { master })
    }

    /// Keep what the pane reads of `child`'s job.
    #[cfg(windows)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the same signature as the Unix form, which duplicates a descriptor"
    )]
    pub(crate) fn of(_master: &ngterm_pty::Master, child: &ngterm_pty::Child) -> Result<Self, PaneError> {
        Ok(Self {
            child: (child.pid(), child.job_view()),
        })
    }

    /// The PTY's foreground process group (`tcgetpgrp`), the pid of its leader; `None` when the PTY cannot say (the
    /// child side closed).
    #[cfg(unix)]
    pub(crate) fn foreground_pid(&self) -> Option<u32> {
        use std::os::fd::AsRawFd;

        // SAFETY: `tcgetpgrp` reads the process group of the master this struct owns; no memory is passed.
        let pgrp = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        u32::try_from(pgrp).ok().filter(|pgrp| *pgrp > 0)
    }

    /// What stands in for the foreground process group on Windows, whose pseudoconsole has none: of the processes in
    /// the program's job (its console's: an MSYS program's parent, an MSYS fork, is gone by the time it runs, so the
    /// tree of parents would miss it), the oldest program started since the pane's own, else that program (a shell
    /// at its prompt), or that program when it is `cmd.exe /D /S /C` running a command line, which has no job control
    /// (`ngterm_session::process_table::console_foreground` has the rule and why). The job was chosen over
    /// `GetConsoleProcessList`, which only a process attached to the console may call (a helper per read, or the
    /// app's one console slot) and which orders nothing either. `None` once the program ended.
    #[cfg(windows)]
    pub(crate) fn foreground_pid(&self) -> Option<u32> {
        let (root, job) = &self.child;
        ngterm_session::process_table::job_foreground(*root, &job.process_ids()?)
    }

    /// Canonical input with echo off (`tcgetattr`): the program reads a password. `false` when the PTY cannot say,
    /// and always on Windows.
    #[cfg(unix)]
    pub(crate) fn password_input(&self) -> bool {
        use std::os::fd::AsRawFd;

        let mut termios = std::mem::MaybeUninit::<libc::termios>::zeroed();
        // SAFETY: the descriptor is the master this struct owns; `tcgetattr` only writes a `termios` into the buffer,
        // which is large enough and zeroed (a valid `termios`) whether or not it succeeds.
        let read = unsafe { libc::tcgetattr(self.master.as_raw_fd(), termios.as_mut_ptr()) };
        if read != 0 {
            return false;
        }
        // SAFETY: zero-initialized above and filled by the successful `tcgetattr`.
        let modes = unsafe { termios.assume_init() }.c_lflag;
        modes & libc::ICANON != 0 && modes & libc::ECHO == 0
    }

    #[cfg(windows)]
    #[expect(
        clippy::unused_self,
        reason = "the same signature as the Unix form, which reads the master"
    )]
    pub(crate) fn password_input(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_becomes_the_ptys_size() {
        let size = GridSize {
            cols: 80,
            rows: 24,
            cell_width: 10,
            cell_height: 20,
        };
        assert_eq!(size.pty().pixels(), (800, 480));
        assert_eq!((size.pty().rows, size.pty().cols), (24, 80));
    }

    #[test]
    fn write_errors_map_to_the_panes() {
        assert_eq!(PaneError::from(WriteError::Backpressure), PaneError::Backpressure);
        assert_eq!(PaneError::from(WriteError::Closed), PaneError::Closed);
    }
}
