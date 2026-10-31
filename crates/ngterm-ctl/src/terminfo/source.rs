//! The entry's source text, decompiled from its compiled form in Rust as `infocmp -x <entry>` prints it, so the install
//! needs no local `infocmp` (Windows has none) and reads the same text on every OS. The remote `tic` compiles it back
//! into the same entry.
//!
//! Ported from ncurses 6.6 (the `ncurses` notice in `NOTICE`), modified: reduced to what `infocmp -x` runs with its
//! defaults (terminfo output sorted by name, 60 columns, no `-f`, `-q`, `-1` or `-W`). The reader is
//! `ncurses/tinfo/read_entry.c`'s `_nc_read_termtype` (both the legacy and the 32-bit number format, with the extended
//! section `tic -x` writes); the text is `progs/dump_entry.c`'s `fmt_entry`, `wrap_concat`, `number_format` and
//! `show_entry`, `ncurses/tinfo/comp_expand.c`'s `_nc_tic_expand` for string values, and `progs/dump_entry.c`'s
//! `repair_acsc`, which `infocmp` runs on each entry it reads. The comment line names `ngtermctl` where `infocmp`
//! names itself.

use std::fmt::Write as _;
use std::path::Path;

use super::names::{BOOLEANS, NUMBERS, STRINGS};

/// The legacy format's magic (`MAGIC`, 0432): numbers are 16 bits.
const MAGIC: u16 = 0o432;
/// The 32-bit number format's magic (`MAGIC2`, 01036).
const MAGIC_WIDE: u16 = 0o1036;

/// A string capability: its bytes, or cancelled (`name@`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    Cancelled,
    Value(Vec<u8>),
}

/// One group of capabilities, as `(name, value)` pairs of those present: booleans that are not false, numbers that
/// are not absent, strings that are not absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub booleans: Vec<(String, i8)>,
    pub numbers: Vec<(String, i32)>,
    pub strings: Vec<(String, Text)>,
}

/// A compiled entry: its names line, its standard capabilities and its extended ones (`tic -x`), each in stored order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub names: Vec<u8>,
    pub standard: Capabilities,
    pub extended: Capabilities,
}

/// Why a compiled entry could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corrupt {
    /// The file does not start with either magic number.
    NotCompiled,
    /// The file ends before the data its header announces.
    Truncated,
    /// A count, offset or name is out of range.
    Invalid,
}

impl Corrupt {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NotCompiled => "not a compiled terminfo entry",
            Self::Truncated => "the compiled entry is truncated",
            Self::Invalid => "the compiled entry is corrupt",
        }
    }
}

/// A read position in the file (`fake_read` and its macros).
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Corrupt> {
        let end = self.at.checked_add(count).ok_or(Corrupt::Invalid)?;
        let taken = self.bytes.get(self.at..end).ok_or(Corrupt::Truncated)?;
        self.at = end;
        Ok(taken)
    }

    /// `count` little-endian shorts, signed as `MyNumber` reads them.
    fn shorts(&mut self, count: usize) -> Result<Vec<i16>, Corrupt> {
        let bytes = self.take(count.checked_mul(2).ok_or(Corrupt::Invalid)?)?;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect())
    }

    /// A header of `N` counts: a negative one is corrupt.
    fn counts<const N: usize>(&mut self) -> Result<[usize; N], Corrupt> {
        let mut counts = [0; N];
        for (count, value) in counts.iter_mut().zip(self.shorts(N)?) {
            *count = usize::try_from(value).map_err(|_| Corrupt::Invalid)?;
        }
        Ok(counts)
    }

    /// `even_boundary`: skips the pad byte after an odd-sized part, if the file has it.
    fn align(&mut self, size: usize) {
        if !size.is_multiple_of(2) && self.at < self.bytes.len() {
            self.at += 1;
        }
    }

    fn numbers(&mut self, count: usize, wide: bool) -> Result<Vec<i32>, Corrupt> {
        let size = if wide { 4 } else { 2 };
        let bytes = self.take(count.checked_mul(size).ok_or(Corrupt::Invalid)?)?;
        Ok(if wide {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| i32::from_le_bytes(*word))
                .collect()
        } else {
            // 16-bit values are sign-extended, so -1 and -2 keep their meaning
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| i32::from(i16::from_le_bytes(*pair)))
                .collect()
        })
    }
}

/// A string offset's value in `table` (`convert_strings`): `None` when absent (-1, past the table, or without a NUL),
/// cancelled at -2. `required` refuses an absent or empty value, as the extended names are.
fn string_at(offset: i16, table: &[u8], required: bool) -> Result<Option<Text>, Corrupt> {
    let text = match offset {
        -1 => None,
        -2 => Some(Text::Cancelled),
        _ if usize::try_from(offset).is_ok_and(|start| start > table.len()) => None,
        _ => {
            let start = usize::try_from(offset)
                .ok()
                .filter(|start| *start < table.len())
                .ok_or(Corrupt::Invalid)?;
            table[start..]
                .iter()
                .position(|byte| *byte == 0)
                .map(|length| Text::Value(table[start..start + length].to_vec()))
        }
    };
    match text {
        Some(Text::Value(value)) if !(required && value.is_empty()) => Ok(Some(Text::Value(value))),
        Some(Text::Cancelled) if !required => Ok(Some(Text::Cancelled)),
        None if !required => Ok(None),
        _ => Err(Corrupt::Invalid),
    }
}

/// The present booleans and numbers among `values`, named by `names` (the standard ones by index).
fn present<T: Copy>(names: &[impl AsRef<str>], values: &[T], absent: impl Fn(T) -> bool) -> Vec<(String, T)> {
    names
        .iter()
        .zip(values)
        .filter(|(_, value)| !absent(**value))
        .map(|(name, value)| (name.as_ref().to_owned(), *value))
        .collect()
}

/// Reads a compiled entry (`_nc_read_termtype`, with `use_extended_names(TRUE)` as `-x` sets).
pub fn read(bytes: &[u8]) -> Result<Entry, Corrupt> {
    let mut cursor = Cursor { bytes, at: 0 };
    let magic = cursor.take(2).map_err(|_| Corrupt::NotCompiled)?;
    let wide = match u16::from_le_bytes([magic[0], magic[1]]) {
        MAGIC => false,
        MAGIC_WIDE => true,
        _ => return Err(Corrupt::NotCompiled),
    };
    let [name_size, bool_count, num_count, str_count, str_size] = cursor.counts()?;
    if bool_count > BOOLEANS.len() || num_count > NUMBERS.len() || str_count > STRINGS.len() {
        return Err(Corrupt::Invalid);
    }
    let names = cursor.take(name_size)?;
    let names = names[..names.iter().position(|byte| *byte == 0).unwrap_or(names.len())].to_vec();
    let booleans: Vec<i8> = cursor.take(bool_count)?.iter().map(|byte| byte.cast_signed()).collect();
    cursor.align(name_size + bool_count);
    let numbers = cursor.numbers(num_count, wide)?;
    let mut strings = Vec::new();
    if str_count > 0 {
        let offsets = cursor.shorts(str_count)?;
        let table = cursor.take(str_size)?;
        for (name, offset) in STRINGS.iter().zip(offsets) {
            if let Some(text) = string_at(offset, table, false)? {
                strings.push(((*name).to_owned(), text));
            }
        }
    }
    let mut standard = Capabilities {
        booleans: present(&BOOLEANS, &booleans, |value| value == 0),
        numbers: present(&NUMBERS, &numbers, |value| value == -1),
        strings,
    };
    repair_acsc(&mut standard.strings);
    cursor.align(str_size);
    let extended = read_extended(&mut cursor, wide)?;
    Ok(Entry {
        names,
        standard,
        extended,
    })
}

/// The extended section after the standard data (`tic -x`'s user-defined capabilities), when its header has any
/// count above zero (`valid_shorts`).
fn read_extended(cursor: &mut Cursor<'_>, wide: bool) -> Result<Capabilities, Corrupt> {
    let start = cursor.at;
    match cursor.shorts(5) {
        Ok(header) if header.iter().any(|value| *value > 0) => cursor.at = start,
        _ => return Ok(Capabilities::default()),
    }
    let [bool_count, num_count, str_count, _, table_size] = cursor.counts()?;
    let booleans: Vec<i8> = cursor.take(bool_count)?.iter().map(|byte| byte.cast_signed()).collect();
    cursor.align(bool_count);
    let numbers = cursor.numbers(num_count, wide)?;
    let name_count = bool_count + num_count + str_count;
    let offsets = cursor.shorts(str_count + name_count)?;
    let table = cursor.take(table_size)?;
    let (value_offsets, name_offsets) = offsets.split_at(str_count);
    let values = value_offsets
        .iter()
        .map(|offset| string_at(*offset, table, false))
        .collect::<Result<Vec<_>, _>>()?;
    // the names follow the values that are present, each with its NUL
    let base: usize = values
        .iter()
        .map(|value| match value {
            Some(Text::Value(bytes)) => bytes.len() + 1,
            _ => 0,
        })
        .sum();
    let names_table = table.get(base..).ok_or(Corrupt::Invalid)?;
    let names = name_offsets
        .iter()
        .map(|offset| match string_at(*offset, names_table, true)? {
            Some(Text::Value(name)) => Ok(String::from_utf8_lossy(&name).into_owned()),
            _ => Err(Corrupt::Invalid),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (bool_names, rest) = names.split_at(bool_count);
    let (num_names, str_names) = rest.split_at(num_count);
    Ok(Capabilities {
        booleans: present(bool_names, &booleans, |value| value == 0),
        numbers: present(num_names, &numbers, |value| value == -1),
        strings: str_names
            .iter()
            .zip(values)
            .filter_map(|(name, value)| value.map(|text| (name.clone(), text)))
            .collect(),
    })
}

/// `repair_acsc`: an `acsc` whose pairs are not in ascending order is sorted by their first character, a later pair
/// for the same character winning and an unpaired last character kept at the end.
fn repair_acsc(strings: &mut [(String, Text)]) {
    let Some((_, Text::Value(acsc))) = strings.iter_mut().find(|(name, _)| name == "acsc") else {
        return;
    };
    let ascending = acsc
        .chunks(2)
        .map(|pair| pair[0])
        .try_fold(0_u8, |previous, first| (previous < first).then_some(first))
        .is_some();
    if ascending {
        return;
    }
    let mut mapped = [None; 256];
    let mut extra = None;
    for pair in acsc.chunks(2) {
        match *pair {
            [source, target] => mapped[usize::from(source)] = Some(target),
            [source] => extra = Some(source),
            _ => {}
        }
    }
    let mut repaired: Vec<u8> = (0..=u8::MAX)
        .zip(mapped)
        .filter_map(|(source, target)| target.map(|target| [source, target]))
        .flatten()
        .collect();
    repaired.extend(extra);
    *acsc = repaired;
}

/// The line width `infocmp` wraps at (`-w`'s default), the indent of a continuation line, and what follows each
/// capability.
const WIDTH: usize = 60;
const INDENT: usize = 8;
const SEPARATOR: &[u8] = b", ";
const TRAILER: &[u8] = b"\n\t";

/// The text being laid out (`outbuf`) and its column, which counts the capabilities but not the separators.
#[derive(Default)]
struct Layout {
    text: Vec<u8>,
    column: usize,
    wrote: bool,
}

impl Layout {
    /// `force_wrap`: ends the line, its trailing spaces trimmed, and starts an indented one.
    fn wrap(&mut self) {
        while self.text.last() == Some(&b' ') {
            self.text.pop();
        }
        self.text.extend_from_slice(TRAILER);
        self.column = INDENT;
    }

    /// `wrap_concat1`: one capability, on a new line when it would pass the width and the line has one already.
    fn push(&mut self, capability: &[u8]) {
        if self.column > INDENT && self.column + SEPARATOR.len() + capability.len() > WIDTH {
            self.wrap();
        }
        self.text.extend_from_slice(capability);
        self.text.extend_from_slice(SEPARATOR);
        self.column += capability.len();
        self.wrote = true;
    }

    /// Each group starts on a line of its own.
    fn end_group(&mut self) {
        if self.column != INDENT {
            self.wrap();
        }
    }

    /// The kludge at the end of `fmt_entry` and `show_entry`'s trim: no empty last line, no trailing space, the last
    /// capability's comma kept, and one newline.
    fn finish(mut self) -> Vec<u8> {
        if self.wrote && self.text.ends_with(TRAILER) {
            self.text.truncate(self.text.len() - TRAILER.len());
            self.text.push(b' ');
        }
        let mut used = self.text.len();
        for at in (1..self.text.len()).rev() {
            match self.text[at] {
                b'\n' => {}
                byte if byte.is_ascii_whitespace() || byte == 0x0b => used = at,
                b',' if self.text[at - 1] != b'\\' => used = at + 1,
                _ => break,
            }
        }
        self.text.truncate(used);
        self.text.push(b'\n');
        self.text
    }
}

/// `number_format`: a value above 255 within 16 of a power of two is shown in hex.
fn number(value: i32) -> String {
    let near_power_of_two = u64::try_from(value).is_ok_and(|value| {
        value > 255 && (8..64).any(|bits| (1_u64 << bits) - 16 <= value && value < (1_u64 << bits) + 16)
    });
    if near_power_of_two {
        format!("{value:#x}")
    } else {
        value.to_string()
    }
}

/// The source text: a comment naming the file, then the entry as `infocmp -x` lays it out.
#[must_use]
pub fn source(entry: &Entry, file: &Path) -> Vec<u8> {
    let mut layout = Layout::default();
    layout.text.extend_from_slice(&entry.names);
    layout.text.extend_from_slice(SEPARATOR);
    layout.column = layout.text.len();
    layout.wrap();
    for (name, value) in sorted(&entry.standard.booleans).chain(&entry.extended.booleans) {
        let cancelled = if *value < 0 { "@" } else { "" };
        layout.push(format!("{name}{cancelled}").as_bytes());
    }
    layout.end_group();
    for (name, value) in sorted(&entry.standard.numbers).chain(&entry.extended.numbers) {
        let text = if *value < 0 {
            format!("{name}@")
        } else {
            format!("{name}#{}", number(*value))
        };
        layout.push(text.as_bytes());
    }
    layout.end_group();
    for (name, text) in sorted(&entry.standard.strings).chain(&entry.extended.strings) {
        let text = match text {
            Text::Cancelled => format!("{name}@"),
            Text::Value(value) => format!("{name}={}", expand(value)),
        };
        layout.push(text.as_bytes());
    }
    let mut out = format!("#\tReconstructed via ngtermctl from file: {}\n", file.display()).into_bytes();
    out.extend(layout.finish());
    out
}

/// A standard group in `infocmp`'s default order: by name, in the C locale.
fn sorted<T>(group: &[(String, T)]) -> impl Iterator<Item = &(String, T)> {
    let mut sorted: Vec<&(String, T)> = group.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    sorted.into_iter()
}

/// `isprint` in the C locale, below DEL (`REALPRINT`).
const fn printable(byte: u8) -> bool {
    matches!(byte, b' '..=b'~')
}

/// At most this many control characters are noted for the short-string fix-up (`MAX_TC_FIXUPS`); a value whose other
/// text is shorter than `MIN_TC_FIXUPS` shows them as `^X` rather than octal.
const MAX_FIXUPS: usize = 10;
const MIN_FIXUPS: usize = 4;

/// A string value as source (`_nc_tic_expand` in tic format, numbers left as they are).
fn expand(value: &[u8]) -> String {
    let mut out = String::new();
    // where each noted control character's octal form starts, and the character
    let mut fixups: Vec<(usize, u8)> = Vec::new();
    let mut at = 0;
    while at < value.len() {
        let byte = value[at];
        let next = value.get(at + 1).copied();
        if byte == b'%' && next.is_some_and(printable) {
            out.push('%');
            if next == Some(b',') {
                out.push('\\');
            }
            out.push(char::from(next.unwrap_or_default()));
            at += 2;
            continue;
        }
        let first = at == 0;
        let previous = if first { None } else { Some(value[at - 1]) };
        if !expand_plain(&mut out, byte, previous, &value[at..]) {
            if fixups.len() < MAX_FIXUPS && (byte == 0x7f || byte < 0x20) {
                fixups.push((out.len(), byte));
            }
            let _ = write!(out, "\\{byte:03o}");
        }
        at += 1;
    }
    if !fixups.is_empty() && out.len() - 4 * fixups.len() < MIN_FIXUPS {
        for (start, byte) in fixups.into_iter().rev() {
            let caret = if byte == 0x7f { '?' } else { char::from(byte + b'@') };
            out.replace_range(start..start + 4, &format!("^{caret}"));
        }
    }
    out
}

/// One byte of a value in its escaped or plain form, or `false` for a byte that takes the octal form. `rest` starts
/// at the byte.
fn expand_plain(out: &mut String, byte: u8, previous: Option<u8>, rest: &[u8]) -> bool {
    let next = rest.get(1).copied().unwrap_or_default();
    let escaped = match byte {
        0x80 => "\\0",
        0x1b => "\\E",
        b'\\' if previous != Some(b'^') => "\\\\",
        b' ' if previous.is_none() || rest.iter().all(|byte| *byte == b' ') => "\\s",
        b',' => "\\,",
        b'^' => "\\^",
        b'\r' => "\\r",
        b'\n' => "\\n",
        _ if printable(byte) => {
            out.push(char::from(byte));
            return true;
        }
        _ if byte < 0x20 && next.is_ascii_digit() => {
            out.push('^');
            out.push(char::from(byte + b'@'));
            return true;
        }
        _ => return false,
    };
    out.push_str(escaped);
    true
}

/// The decompiler's tests and the compiled-entry writer the install's tests use too.
#[cfg(test)]
pub(super) mod tests;
