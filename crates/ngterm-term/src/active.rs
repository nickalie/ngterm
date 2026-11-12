//! Reading the program's screen as it stands ([`Pane::active_screen`]): the active area, row by row, whatever the
//! pane's views show of it.
//!
//! A [`Frame`] from [`Pane::frame`] is what a view draws: the viewport, scrolled back or not, its rows marked with the
//! selection, and while the program holds its output (synchronized output, mode 2026) the screen from before its
//! update. A reader that acts on what the program shows (a key chosen from its screen) needs the screen itself: the
//! active area, read under the VT lock together with the screen generation that [`Pane::key_if`] and
//! [`Pane::paste_if`] check, so nothing the viewport, a selection or a search does can make the rows read differ
//! from the screen the keys reach. The read goes cell by cell through libghostty-vt's grid references, which leave
//! the paint's render state and its dirty flags alone.

use std::sync::Arc;

use ngterm_ghostty_vt as vt;

use crate::PaneError;
use crate::pane::{Pane, lock};
use crate::render::{Colors, Cursor, CursorPosition, Frame, grid_row, rgb};

/// The program's screen as it stood at [`ActiveScreen::generation`] ([`Pane::active_screen`]).
#[derive(Debug, Clone)]
pub struct ActiveScreen {
    /// The active area's rows top to bottom, the cursor where it stands in them (visible as DECTCEM says) and the
    /// colors the cells resolve against. No row carries a selection, the frame no Kitty images,
    /// and its `seq` is 0: it is no paint's frame.
    pub frame: Frame,
    /// The screen generation the rows were read at: [`Pane::key_if`] and [`Pane::paste_if`] send only while the
    /// screen is still this one. Neither scrolling the viewport nor a selection or a search changes it; the program's
    /// output, a resize, a reset or a clear does.
    pub generation: u64,
    /// The pane was resized to a new column or row count and the program has written nothing since: the rows are the
    /// VT's reflow of the last screen the program drew, not one it drew for this grid. The same grid again, or a new
    /// cell size alone, leaves this as it was.
    pub reflowed: bool,
}

/// DECTCEM, the cursor's visibility.
const CURSOR_VISIBLE: vt::Mode = vt::Mode::dec(25);

impl Pane {
    /// The program's screen as it stands: the active area (not the viewport, which may be scrolled back), with the
    /// screen generation it was read at, in one hold of the VT lock. `None` while the program holds its output
    /// (synchronized output, mode 2026): the screen is then half way through an update. A hold past its timeout is
    /// ended first, as a paint ends it.
    ///
    /// # Errors
    ///
    /// [`PaneError::Terminal`] when the library fails to read a cell.
    pub fn active_screen(&self) -> Result<Option<ActiveScreen>, PaneError> {
        let mut vt = lock(&self.shared().vt);
        if self.shared().holding(&mut vt) {
            return Ok(None);
        }
        let terminal = &vt.terminal;
        let rows = (0..terminal.rows())
            .map(|y| terminal.active_row(y))
            .collect::<Result<Vec<_>, _>>()
            .map_err(PaneError::terminal)?;
        let cols = terminal.cols();
        let (column, row) = terminal.cursor();
        let visible = terminal.mode(CURSOR_VISIBLE);
        let engine = terminal.colors();
        let (generation, reflowed) = (vt.generation, vt.reflowed);
        drop(vt);
        let defaults = Colors::default();
        let colors = Colors {
            background: engine.background.map_or(defaults.background, rgb),
            foreground: engine.foreground.map_or(defaults.foreground, rgb),
            cursor: engine.cursor.map(rgb),
            palette: Box::new(engine.palette.map(rgb)),
        };
        let frame = Frame {
            seq: 0,
            cols,
            rows: rows
                .into_iter()
                .map(|row| Arc::new(grid_row(row, 0, &colors.palette)))
                .collect(),
            cursor: Cursor {
                position: Some(CursorPosition {
                    column,
                    row,
                    wide_tail: false,
                }),
                visible,
                ..Cursor::default()
            },
            colors: Arc::new(colors),
            ..Frame::default()
        };
        Ok(Some(ActiveScreen {
            frame,
            generation,
            reflowed,
        }))
    }
}
