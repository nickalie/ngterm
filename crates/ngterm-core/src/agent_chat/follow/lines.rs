//! Splitting a transcript into lines, forward ([`Splitter`]) and backward ([`BackScan`]), never holding more of one
//! line than [`LINE_CAP`]: a longer line is kept as its offset alone and becomes `unknown{oversized}`.

use std::io::{self, Read, Seek, SeekFrom};
use std::mem;

use crate::agent_chat::event::LINE_CAP;

/// The most bytes one read of a backward scan takes, however far a step goes.
const READ_CAP: u64 = 1024 * 1024;

/// A complete line: its byte offset, and its bytes without the newline, or `None` for a line over [`LINE_CAP`].
#[derive(Debug)]
pub(crate) struct RawLine {
    pub cursor: u64,
    pub bytes: Option<Vec<u8>>,
}

/// A line read in pieces, held until it passes [`LINE_CAP`].
#[derive(Debug, Default)]
struct Partial {
    bytes: Vec<u8>,
    oversized: bool,
}

impl Partial {
    fn append(&mut self, piece: &[u8]) {
        if self.fits(piece) {
            self.bytes.extend_from_slice(piece);
        }
    }

    fn prepend(&mut self, piece: &[u8]) {
        if self.fits(piece) {
            self.bytes.splice(0..0, piece.iter().copied());
        }
    }

    /// Whether `piece` can join the line: once the line passes the cap, its bytes are dropped and nothing joins.
    fn fits(&mut self, piece: &[u8]) -> bool {
        if !self.oversized && self.bytes.len() + piece.len() > LINE_CAP {
            self.oversized = true;
            self.bytes = Vec::new();
        }
        !self.oversized
    }

    /// The line, complete, starting at `cursor`; the next one starts empty.
    fn take(&mut self, cursor: u64) -> RawLine {
        let bytes = (!self.oversized).then(|| mem::take(&mut self.bytes));
        *self = Self::default();
        RawLine { cursor, bytes }
    }
}

/// Splits bytes read forward into lines, buffering the partial last one until its newline.
#[derive(Debug)]
pub(crate) struct Splitter {
    /// The offset of the next byte to read.
    offset: u64,
    /// The offset of the line being buffered.
    start: u64,
    partial: Partial,
}

impl Splitter {
    /// A splitter reading from `offset`, a line's start.
    pub fn at(offset: u64) -> Self {
        Self {
            offset,
            start: offset,
            partial: Partial::default(),
        }
    }

    /// The offset of the next byte to read.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The offset of the line being buffered: where the lines not read in full start.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// The bytes held for the partial line.
    #[cfg(test)]
    pub fn buffered(&self) -> usize {
        self.partial.bytes.capacity()
    }

    /// The lines `bytes`, read at [`Splitter::offset`], complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<RawLine> {
        let mut lines = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|&byte| byte == b'\n') {
            self.partial.append(&rest[..newline]);
            self.offset += newline as u64 + 1;
            lines.push(self.partial.take(self.start));
            self.start = self.offset;
            rest = &rest[newline + 1..];
        }
        self.partial.append(rest);
        self.offset += rest.len() as u64;
        lines
    }
}

/// The complete lines that end at or before an offset, read backward a step at a time.
///
/// The bytes after the last newline before that offset are skipped: a line still being written, or the rest of the
/// line the offset falls in.
#[derive(Debug)]
pub(crate) struct BackScan {
    end: u64,
    /// Read down to here.
    pos: u64,
    /// A newline was found: the bytes read now belong to a complete line.
    collecting: bool,
    partial: Partial,
    /// Newest first.
    lines: Vec<RawLine>,
    /// Where the lines after these start, once known.
    next: Option<u64>,
}

impl BackScan {
    pub fn new(end: u64) -> Self {
        Self {
            end,
            pos: end,
            collecting: false,
            partial: Partial::default(),
            lines: Vec::new(),
            next: (end == 0).then_some(0),
        }
    }

    /// The file's start was reached: every line before the end is read.
    pub fn done(&self) -> bool {
        self.pos == 0
    }

    /// Where the lines after these start: following resumes there.
    pub fn next(&self) -> Option<u64> {
        self.next
    }

    /// The bytes read so far.
    pub fn read_bytes(&self) -> u64 {
        self.end - self.pos
    }

    /// The lines read, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &RawLine> {
        self.lines.iter().rev()
    }

    /// Reads `step` more bytes backward (fewer at the file's start), in reads of at most [`READ_CAP`].
    pub fn read<R: Read + Seek>(&mut self, reader: &mut R, step: u64) -> io::Result<()> {
        let floor = self.pos.saturating_sub(step);
        while self.pos > floor {
            let from = self.pos.saturating_sub(READ_CAP).max(floor);
            let mut chunk = vec![0; usize::try_from(self.pos - from).unwrap_or(0)];
            reader.seek(SeekFrom::Start(from))?;
            reader.read_exact(&mut chunk)?;
            self.take_chunk(&chunk, from);
        }
        Ok(())
    }

    /// Takes the bytes just before those read so far, read at `from`.
    fn take_chunk(&mut self, chunk: &[u8], from: u64) {
        let mut upto = chunk.len();
        while let Some(newline) = chunk[..upto].iter().rposition(|&byte| byte == b'\n') {
            self.newline(&chunk[newline + 1..upto], from + newline as u64 + 1);
            upto = newline;
        }
        if self.collecting {
            self.partial.prepend(&chunk[..upto]);
        }
        self.pos = from;
        if from == 0 {
            if self.collecting {
                self.lines.push(self.partial.take(0));
            }
            self.next.get_or_insert(0);
        }
    }

    /// A newline ends just before `start`; `piece` is what follows it in this chunk.
    fn newline(&mut self, piece: &[u8], start: u64) {
        if self.collecting {
            self.partial.prepend(piece);
            self.lines.push(self.partial.take(start));
        } else {
            self.next = Some(start);
            self.collecting = true;
        }
    }
}
