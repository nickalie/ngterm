//! Reading cells by their point (`grid_ref.h`): a row of the active area as the program left it, whatever the
//! viewport shows (scrolled back or not) and whatever the selection or a search covers.
//!
//! Each read resolves its points to untracked grid references and copies what they hold within one borrow of the
//! terminal, so no reference outlives the next mutating call. The library warns that grid references are no render
//! loop: a renderer reads a [`crate::RenderState`] instead (whose dirty flags this read leaves alone).

use crate::cell::{CellWidth, Style};
use crate::selection::{Point, PointTag};
use crate::{Error, Terminal, ffi};

/// One cell, read by its point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridCell {
    /// The grapheme cluster; empty for a cell without text.
    pub text: String,
    pub width: CellWidth,
    /// The cell's style (the default one for a cell with none).
    pub style: Style,
    /// Part of an OSC 8 hyperlink.
    pub hyperlink: bool,
}

/// One row, read by its points.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridRow {
    /// Left to right, one per column.
    pub cells: Vec<GridCell>,
    /// The row soft-wraps: its text continues on the next row.
    pub wrapped: bool,
}

impl Terminal {
    /// Row `y` of the active area (0 is its top row), as the program left it: the viewport's scroll position, the
    /// selection and a search play no part.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidValue`] for a row past the active area; the library's result when a cell cannot be read.
    pub fn active_row(&self, y: u16) -> Result<GridRow, Error> {
        if y >= self.rows() {
            return Err(Error::InvalidValue);
        }
        let cols = self.cols();
        let mut row = GridRow {
            cells: Vec::with_capacity(usize::from(cols)),
            wrapped: false,
        };
        for x in 0..cols {
            let grid_ref = self.grid_ref(Point::new(PointTag::Active, x, u32::from(y)))?;
            if x == 0 {
                row.wrapped = wrapped(&grid_ref)?;
            }
            row.cells.push(cell(&grid_ref)?);
        }
        Ok(row)
    }
}

/// Whether the row of `grid_ref` (just resolved, the terminal still borrowed) soft-wraps.
fn wrapped(grid_ref: &ffi::GhosttyGridRef) -> Result<bool, Error> {
    let mut row: ffi::GhosttyRow = 0;
    // SAFETY: the reference was resolved under the caller's borrow of the terminal, with no mutation since; `row` is
    // a valid out-pointer.
    Error::check(unsafe { ffi::ghostty_grid_ref_row(grid_ref, &raw mut row) })?;
    let mut wrap = false;
    // SAFETY: `row` is a row value; `ROW_DATA_WRAP` writes a `bool`.
    Error::check(unsafe { ffi::ghostty_row_get(row, ffi::GHOSTTY_ROW_DATA_WRAP, (&raw mut wrap).cast()) })?;
    Ok(wrap)
}

/// The cell at `grid_ref` (just resolved, the terminal still borrowed).
fn cell(grid_ref: &ffi::GhosttyGridRef) -> Result<GridCell, Error> {
    let mut raw: ffi::GhosttyCell = 0;
    // SAFETY: the reference was resolved under the caller's borrow of the terminal, with no mutation since; `raw` is
    // a valid out-pointer.
    Error::check(unsafe { ffi::ghostty_grid_ref_cell(grid_ref, &raw mut raw) })?;
    let mut wide: ffi::GhosttyCellWide = ffi::GHOSTTY_CELL_WIDE_NARROW;
    // SAFETY: `raw` is a cell value; `CELL_DATA_WIDE` writes a `GhosttyCellWide`.
    Error::check(unsafe { ffi::ghostty_cell_get(raw, ffi::GHOSTTY_CELL_DATA_WIDE, (&raw mut wide).cast()) })?;
    let (mut styled, mut hyperlink) = (false, false);
    // SAFETY: `CELL_DATA_HAS_STYLING` and `CELL_DATA_HAS_HYPERLINK` each write a `bool`.
    Error::check(unsafe { ffi::ghostty_cell_get(raw, ffi::GHOSTTY_CELL_DATA_HAS_STYLING, (&raw mut styled).cast()) })?;
    // SAFETY: as above.
    Error::check(unsafe {
        ffi::ghostty_cell_get(raw, ffi::GHOSTTY_CELL_DATA_HAS_HYPERLINK, (&raw mut hyperlink).cast())
    })?;
    Ok(GridCell {
        text: graphemes(grid_ref)?,
        width: match wide {
            ffi::GHOSTTY_CELL_WIDE_WIDE => CellWidth::Wide,
            ffi::GHOSTTY_CELL_WIDE_SPACER_TAIL => CellWidth::SpacerTail,
            ffi::GHOSTTY_CELL_WIDE_SPACER_HEAD => CellWidth::SpacerHead,
            _ => CellWidth::Narrow,
        },
        style: if styled { style(grid_ref)? } else { Style::default() },
        hyperlink,
    })
}

/// The style of the cell at `grid_ref`.
fn style(grid_ref: &ffi::GhosttyGridRef) -> Result<Style, Error> {
    // SAFETY: an all-zero `GhosttyStyle` is a valid value (size 0, no colors, no flags), sized right below.
    let mut raw: ffi::GhosttyStyle = unsafe { std::mem::zeroed() };
    raw.size = std::mem::size_of::<ffi::GhosttyStyle>();
    // SAFETY: the reference is live (see `cell`); `raw` is a sized out-struct the call fills.
    Error::check(unsafe { ffi::ghostty_grid_ref_style(grid_ref, &raw mut raw) })?;
    Ok(Style::from_raw(&raw))
}

/// The grapheme cluster of the cell at `grid_ref`, as UTF-8.
fn graphemes(grid_ref: &ffi::GhosttyGridRef) -> Result<String, Error> {
    let mut stack = [0_u32; 8];
    let mut len = 0;
    // SAFETY: the reference is live (see `cell`); the call writes at most `stack.len()` codepoints and sets `len`, or
    // sets `len` to the count it needs and writes nothing.
    let result = unsafe { ffi::ghostty_grid_ref_graphemes(grid_ref, stack.as_mut_ptr(), stack.len(), &raw mut len) };
    let codepoints = if result == ffi::GHOSTTY_OUT_OF_SPACE {
        let mut heap = vec![0_u32; len];
        // SAFETY: as above, with a heap buffer of the size the library asked for.
        Error::check(unsafe {
            ffi::ghostty_grid_ref_graphemes(grid_ref, heap.as_mut_ptr(), heap.len(), &raw mut len)
        })?;
        heap.truncate(len);
        heap
    } else {
        Error::check(result)?;
        stack[..len.min(stack.len())].to_vec()
    };
    Ok(codepoints.into_iter().filter_map(char::from_u32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scroll, StyleFlags};

    fn text(row: &GridRow) -> String {
        row.cells
            .iter()
            .map(|cell| if cell.text.is_empty() { " " } else { cell.text.as_str() })
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn the_active_area_is_read_whatever_the_viewport_shows() {
        let mut terminal = Terminal::new(10, 3).unwrap();
        terminal.vt_write(
            b"old 1\r\nold 2\r\nold 3\r\nold 4\r\nnew \x1b[2mfaint\x1b[0m\r\n\x1b[8mhid\x1b[0m e\xcc\x81 \xe4\xb8\xad",
        );
        let rows = |terminal: &Terminal| (0..3).map(|y| terminal.active_row(y).unwrap()).collect::<Vec<_>>();
        let live = rows(&terminal);
        assert_eq!(
            live.iter().map(text).collect::<Vec<_>>(),
            ["old 4", "new faint", "hid e\u{301} \u{4e2d}"]
        );
        let faint = &live[1].cells[4];
        assert_eq!(faint.text, "f");
        assert!(faint.style.flags.contains(StyleFlags::FAINT));
        assert!(live[2].cells[0].style.flags.contains(StyleFlags::INVISIBLE));
        assert!(
            live[1].cells[0].style.flags.is_empty(),
            "an unstyled cell reads the default style"
        );
        assert_eq!(live[2].cells[4].text, "e\u{301}", "the whole grapheme cluster");
        assert_eq!(
            (live[2].cells[6].width, live[2].cells[7].width),
            (CellWidth::Wide, CellWidth::SpacerTail)
        );

        // scrolled back to the top, and with everything selected: the same rows
        terminal.scroll_viewport(Scroll::Top);
        assert!(!terminal.viewport_at_bottom());
        assert_eq!(rows(&terminal), live);
        assert!(terminal.select_all().unwrap());
        assert_eq!(rows(&terminal), live);
        assert_eq!(terminal.active_row(3), Err(Error::InvalidValue));
    }

    #[test]
    fn a_soft_wrapped_row_says_so() {
        let mut terminal = Terminal::new(4, 3).unwrap();
        terminal.vt_write(b"abcdef\r\nxy");
        let (first, second) = (terminal.active_row(0).unwrap(), terminal.active_row(1).unwrap());
        assert!(first.wrapped && !second.wrapped);
        assert_eq!((text(&first), text(&second)), ("abcd".to_owned(), "ef".to_owned()));
    }

    #[test]
    fn long_clusters_and_wide_wrap_spacers_are_read_by_point() {
        let cluster = format!("e{}", "\u{301}".repeat(24));
        let mut terminal = Terminal::new(4, 3).unwrap();
        terminal.vt_write(cluster.as_bytes());
        terminal.vt_write("ab中".as_bytes());
        let first = terminal.active_row(0).unwrap();
        assert_eq!(first.cells[0].text, cluster);
        assert_eq!(first.cells[3].width, CellWidth::SpacerHead);
        assert!(first.wrapped);
        let next = terminal.active_row(1).unwrap();
        assert_eq!(next.cells[0].text, "中");
        assert_eq!(next.cells[0].width, CellWidth::Wide);
        assert_eq!(next.cells[1].width, CellWidth::SpacerTail);
        assert!(next.cells[1].text.is_empty());
    }
}
