//! The decompiler against entries written here byte by byte (every OS), and on Linux against the real `tic` and
//! `infocmp`: an entry `tic -x` compiled from a source that reaches every escape, and the system database's entries,
//! each decompiled to exactly the text `infocmp -x` prints, its comment line aside.

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Write as _;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

use super::*;

/// A string slot in a written entry.
#[derive(Clone, Copy)]
pub enum Slot<'a> {
    Absent,
    Cancelled,
    Value(&'a [u8]),
}

/// An extended capability in a written entry.
#[derive(Clone, Copy)]
pub enum Extended<'a> {
    Boolean(i8),
    Number(i32),
    String(Slot<'a>),
}

/// A compiled entry, written as `tic` lays it out (`term(5)`), with 16-bit or (`wide`) 32-bit numbers. The standard
/// capabilities are by index; the extended ones are grouped by type in the order given.
pub struct Compiled<'a> {
    pub names: &'a str,
    pub booleans: &'a [i8],
    pub numbers: &'a [i32],
    pub strings: &'a [Slot<'a>],
    pub extended: &'a [(&'a str, Extended<'a>)],
    pub wide: bool,
}

fn short(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&u16::try_from(value).unwrap().to_le_bytes());
}

fn pad(out: &mut Vec<u8>, size: usize) {
    if !size.is_multiple_of(2) {
        out.push(0);
    }
}

fn numbers(out: &mut Vec<u8>, values: &[i32], wide: bool) {
    for value in values {
        if wide {
            out.extend_from_slice(&value.to_le_bytes());
        } else {
            out.extend_from_slice(&i16::try_from(*value).unwrap().to_le_bytes());
        }
    }
}

/// The offsets and the table of `slots`, offsets counted from `base`.
fn strings(slots: &[Slot<'_>], table: &mut Vec<u8>) -> Vec<i16> {
    slots
        .iter()
        .map(|slot| match slot {
            Slot::Absent => -1,
            Slot::Cancelled => -2,
            Slot::Value(value) => {
                let offset = i16::try_from(table.len()).unwrap();
                table.extend_from_slice(value);
                table.push(0);
                offset
            }
        })
        .collect()
}

impl Compiled<'_> {
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(if self.wide { 0o1036_u16 } else { 0o432 }).to_le_bytes());
        let mut table = Vec::new();
        let offsets = strings(self.strings, &mut table);
        for count in [
            self.names.len() + 1,
            self.booleans.len(),
            self.numbers.len(),
            offsets.len(),
            table.len(),
        ] {
            short(&mut out, count);
        }
        out.extend_from_slice(self.names.as_bytes());
        out.push(0);
        out.extend(self.booleans.iter().map(|value| value.cast_unsigned()));
        pad(&mut out, self.names.len() + 1 + self.booleans.len());
        numbers(&mut out, self.numbers, self.wide);
        out.extend(offsets.iter().flat_map(|offset| offset.to_le_bytes()));
        out.extend_from_slice(&table);
        pad(&mut out, table.len());
        if !self.extended.is_empty() {
            self.write_extended(&mut out);
        }
        out
    }

    fn write_extended(&self, out: &mut Vec<u8>) {
        let of_kind = |kind: u8| {
            self.extended.iter().filter(move |(_, value)| {
                kind == match value {
                    Extended::Boolean(_) => 0,
                    Extended::Number(_) => 1,
                    Extended::String(_) => 2,
                }
            })
        };
        let booleans: Vec<i8> = of_kind(0)
            .filter_map(|(_, value)| match value {
                Extended::Boolean(value) => Some(*value),
                _ => None,
            })
            .collect();
        let values: Vec<i32> = of_kind(1)
            .filter_map(|(_, value)| match value {
                Extended::Number(value) => Some(*value),
                _ => None,
            })
            .collect();
        let slots: Vec<Slot<'_>> = of_kind(2)
            .filter_map(|(_, value)| match value {
                Extended::String(slot) => Some(*slot),
                _ => None,
            })
            .collect();
        let mut table = Vec::new();
        let mut offsets = strings(&slots, &mut table);
        let base = table.len();
        let names: Vec<Slot<'_>> = (0..3)
            .flat_map(|kind| of_kind(kind).map(|(name, _)| Slot::Value(name.as_bytes())))
            .collect();
        let mut names_table = Vec::new();
        offsets.extend(strings(&names, &mut names_table));
        table.extend(names_table);
        let used = offsets.iter().filter(|offset| **offset >= 0).count();
        for count in [booleans.len(), values.len(), slots.len(), used, table.len()] {
            short(out, count);
        }
        out.extend(booleans.iter().map(|value| value.cast_unsigned()));
        pad(out, booleans.len());
        numbers(out, &values, self.wide);
        out.extend(offsets.iter().flat_map(|offset| offset.to_le_bytes()));
        out.extend_from_slice(&table);
        assert!(base <= table.len());
    }
}

/// The text after the comment line.
fn body(source: &[u8]) -> String {
    let text = String::from_utf8(source.to_vec()).unwrap();
    let (comment, body) = text.split_once('\n').unwrap();
    assert!(comment.starts_with("#\tReconstructed via "), "{comment}");
    body.to_owned()
}

fn decompiled(compiled: &Compiled<'_>) -> String {
    body(&source(
        &read(&compiled.bytes()).unwrap(),
        Path::new("/db/x/xterm-ghostty"),
    ))
}

/// `cols` (0), `lines` (2), `colors` (13), `pairs` (14) among the standard numbers.
fn numbers_with(pairs: &[(usize, i32)]) -> Vec<i32> {
    let mut values = vec![-1; 15];
    for (index, value) in pairs {
        values[*index] = *value;
    }
    values
}

fn slots_with<'a>(pairs: &[(&str, Slot<'a>)]) -> Vec<Slot<'a>> {
    let mut slots = vec![Slot::Absent; STRINGS.len()];
    for (name, slot) in pairs {
        let index = STRINGS.iter().position(|known| known == name).unwrap();
        slots[index] = *slot;
    }
    slots
}

#[test]
fn the_names_tables_have_the_compiled_formats_counts_and_order() {
    assert_eq!((BOOLEANS.len(), NUMBERS.len(), STRINGS.len()), (44, 39, 414));
    assert_eq!((BOOLEANS[1], BOOLEANS[28]), ("am", "bce"));
    assert_eq!((NUMBERS[0], NUMBERS[13], NUMBERS[14]), ("cols", "colors", "pairs"));
    assert_eq!((STRINGS[0], STRINGS[146], STRINGS[413]), ("cbt", "acsc", "box1"));
}

#[test]
fn a_small_entry_decompiles_one_group_per_line() {
    let strings = slots_with(&[("bel", Slot::Value(b"\x07")), ("cr", Slot::Value(b"\r"))]);
    let entry = Compiled {
        names: "xterm-ghostty|test",
        booleans: &[0, 1],
        numbers: &numbers_with(&[(0, 80), (2, 24)]),
        strings: &strings,
        extended: &[],
        wide: false,
    };
    assert_eq!(
        decompiled(&entry),
        "xterm-ghostty|test,\n\tam,\n\tcols#80, lines#24,\n\tbel=^G, cr=\\r,\n"
    );
}

#[test]
fn the_comment_line_names_the_file() {
    let entry = Compiled {
        names: "xterm-ghostty",
        booleans: &[],
        numbers: &[80],
        strings: &[],
        extended: &[],
        wide: false,
    };
    let text = source(&read(&entry.bytes()).unwrap(), Path::new("/db/x/xterm-ghostty"));
    assert_eq!(
        String::from_utf8(text).unwrap(),
        "#\tReconstructed via ngtermctl from file: /db/x/xterm-ghostty\nxterm-ghostty,\n\tcols#80,\n"
    );
}

#[test]
fn both_number_formats_read_the_same_entry() {
    let numbers = numbers_with(&[(0, 80), (13, 256), (14, 32767)]);
    let narrow = Compiled {
        names: "t",
        booleans: &[],
        numbers: &numbers,
        strings: &[],
        extended: &[],
        wide: false,
    };
    let wide = Compiled { wide: true, ..narrow };
    assert_eq!(read(&narrow.bytes()), read(&wide.bytes()));
    assert_eq!(decompiled(&wide), "t,\n\tcolors#0x100, cols#80, pairs#0x7fff,\n");
    let large = numbers_with(&[(13, 0x1_0000), (14, 0x1_0000 + 16)]);
    let large = Compiled {
        numbers: &large,
        wide: true,
        ..narrow
    };
    assert_eq!(decompiled(&large), "t,\n\tcolors#0x10000, pairs#65552,\n");
}

#[test]
fn cancelled_capabilities_keep_their_at_sign() {
    let strings = slots_with(&[("rmso", Slot::Cancelled), ("smso", Slot::Value(b"\x1b[7m"))]);
    let entry = Compiled {
        names: "t",
        booleans: &[0, -2],
        numbers: &numbers_with(&[(2, -2)]),
        strings: &strings,
        extended: &[
            ("XT", Extended::String(Slot::Cancelled)),
            ("Ms", Extended::String(Slot::Absent)),
        ],
        wide: false,
    };
    assert_eq!(
        decompiled(&entry),
        "t,\n\tam@,\n\tlines@,\n\trmso@, smso=\\E[7m, XT@,\n"
    );
}

#[test]
fn extended_capabilities_follow_the_standard_ones_in_stored_order() {
    let strings = slots_with(&[("cr", Slot::Value(b"\r"))]);
    let entry = Compiled {
        names: "t",
        booleans: &[0, 1],
        numbers: &numbers_with(&[(0, 80)]),
        strings: &strings,
        extended: &[
            ("Tc", Extended::Boolean(1)),
            ("AX", Extended::Boolean(1)),
            ("No", Extended::Boolean(0)),
            ("Xn", Extended::Number(5)),
            ("Smulx", Extended::String(Slot::Value(b"\x1b[4:%p1%dm"))),
            ("kxIN", Extended::String(Slot::Value(b"\x1b[I"))),
        ],
        wide: true,
    };
    assert_eq!(
        decompiled(&entry),
        "t,\n\tam, Tc, AX,\n\tcols#80, Xn#5,\n\tcr=\\r, Smulx=\\E[4:%p1%dm, kxIN=\\E[I,\n"
    );
}

#[test]
fn lines_wrap_at_sixty_columns_and_a_long_value_stands_alone() {
    let long = format!("\x1b]{}", "x".repeat(70));
    let strings = slots_with(&[
        ("bold", Slot::Value(b"\x1b[1m")),
        ("blink", Slot::Value(b"\x1b[5m")),
        ("civis", Slot::Value(b"\x1b[?25l")),
        ("clear", Slot::Value(b"\x1b[H\x1b[2J")),
        ("cnorm", Slot::Value(b"\x1b[?12l\x1b[?25h")),
        ("csr", Slot::Value(long.as_bytes())),
        ("cub1", Slot::Value(b"\x08")),
    ]);
    let entry = Compiled {
        names: "t",
        booleans: &[],
        numbers: &[],
        strings: &strings,
        extended: &[],
        wide: false,
    };
    assert_eq!(
        decompiled(&entry),
        format!(
            "t,\n\tblink=\\E[5m, bold=\\E[1m, civis=\\E[?25l, clear=\\E[H\\E[2J,\n\tcnorm=\\E[?12l\\E[?25h,\n\
             \tcsr=\\E]{},\n\tcub1=^H,\n",
            "x".repeat(70)
        )
    );
}

#[test]
fn string_values_take_infocmps_escapes() {
    let cases: [(&[u8], &str); 18] = [
        (b"\x07", "^G"),
        (b"\x7f", "^?"),
        (b"\x1b]2;\x07", "\\E]2;\\007"),
        (b"\r\n", "\\r\\n"),
        (b" a b  ", "\\sa b\\s\\s"),
        (b",^\\x", "\\,\\^\\x"),
        (b"a\\b", "a\\\\b"),
        (b"\x80\xff", "\\0\\377"),
        (b"\x01\x012", "^A^A2"),
        (b"%p1%c%%%,", "%p1%c%%%\\,"),
        (b"100%", "100%"),
        (b"\x01\x02\x03\x04", "^A^B^C^D"),
        (b"\x01\x02\x03\x04\x05", "^A^B^C^D^E"),
        (b"\x01\x02\x03\x04!!!!", "\\001\\002\\003\\004!!!!"),
        (b"\x01\x02\x03a", "^A^B^Ca"),
        (b"\x01abcd", "\\001abcd"),
        (b"\x01abc", "^Aabc"),
        (b"\x1b\\", "\\E\\\\"),
    ];
    for (value, expected) in cases {
        assert_eq!(expand(value), expected, "{value:?}");
    }
}

#[test]
fn an_acsc_out_of_order_is_sorted_as_infocmp_repairs_it() {
    let strings = slots_with(&[("acsc", Slot::Value(b"qqaa``xxq"))]);
    let entry = Compiled {
        names: "t",
        booleans: &[],
        numbers: &[],
        strings: &strings,
        extended: &[],
        wide: false,
    };
    assert_eq!(decompiled(&entry), "t,\n\tacsc=``aaqqxxq,\n");
    let ordered = slots_with(&[("acsc", Slot::Value(b"``aaqqxx"))]);
    let entry = Compiled {
        strings: &ordered,
        ..entry
    };
    assert_eq!(decompiled(&entry), "t,\n\tacsc=``aaqqxx,\n");
}

#[test]
fn a_file_that_is_not_a_whole_compiled_entry_is_refused() {
    let entry = Compiled {
        names: "xterm-ghostty|test",
        booleans: &[0, 1],
        numbers: &[80],
        strings: &slots_with(&[("cr", Slot::Value(b"\r"))]),
        extended: &[],
        wide: false,
    }
    .bytes();
    assert_eq!(read(b""), Err(Corrupt::NotCompiled));
    assert_eq!(read(b"xterm-ghostty|test,\n\tcols#80,\n"), Err(Corrupt::NotCompiled));
    assert_eq!(read(&entry[..entry.len() - 3]), Err(Corrupt::Truncated));
    let mut too_many = entry.clone();
    too_many[4..6].copy_from_slice(&45_u16.to_le_bytes());
    assert_eq!(read(&too_many), Err(Corrupt::Invalid));
    let mut negative = entry;
    negative[2..4].copy_from_slice(&(-3_i16).to_le_bytes());
    assert_eq!(read(&negative), Err(Corrupt::Invalid));
}

// -- against the real tic and infocmp (Linux: macOS ships ncurses 5.7, whose infocmp lays numbers out otherwise)

/// `infocmp -x` of `name` in `directory`, its comment line aside; `None` without an `infocmp`.
#[cfg(target_os = "linux")]
fn infocmp(directory: &Path, name: &str) -> Option<String> {
    let infocmp = Path::new("/usr/bin/infocmp");
    if !infocmp.exists() {
        return None;
    }
    let output = Command::new(infocmp)
        .arg("-x")
        .arg("-A")
        .arg(directory)
        .arg(name)
        .env_clear()
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Some(body(&output.stdout))
}

#[cfg(target_os = "linux")]
fn ours(file: &Path) -> String {
    body(&source(&read(&fs::read(file).unwrap()).unwrap(), file))
}

/// What `tic -x` makes of a source that reaches each escape, cancel, group and wrap the decompiler ports.
#[cfg(target_os = "linux")]
const FIXTURE: &str = "xterm-ghostty|ngterm test|Test Entry,\n\
    \tam, bce, xon@, Tc, XT@, AX,\n\
    \tcols#80, colors#0x100, pairs#0x7fff, lines@, it#1000, Xn#5,\n\
    \tbel=^G, kbs=^?, dsl=\\E]2;\\007, acsc=qqaa``xx, cr=\\r, ind=\\n,\n\
    \tis1=\\sa b  , is2=\\,\\^\\\\x\\200\\377\\001\\0012, rmso@, smso=%p1%c%%%\\,,\n\
    \tinitc=\\E]4;%p1%d;rgb:%p2%{255}%*%{1000}%/%2.2X/%p3%{255}%*%{1000}%/%2.2X\\E\\\\,\n\
    \trs1=\\E]\\E\\\\\\Ec, rv=\\E\\\\[[0-9]+;[0-9]+c, u8=\\E[?%[;0123456789]c,\n\
    \tSmulx=\\E[4:%p1%dm, Ms@, kxIN=\\E[I, Sync=\\E[?2026%?%p1%{1}%-%tl%eh%;,\n";

#[test]
#[cfg(target_os = "linux")]
fn an_entry_tic_compiled_decompiles_to_what_infocmp_prints() {
    let tic = Path::new("/usr/bin/tic");
    if !tic.exists() {
        return;
    }
    let database = tempfile::tempdir().unwrap();
    let mut child = Command::new(tic)
        .args(["-x", "-o"])
        .arg(database.path())
        .arg("-")
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(FIXTURE.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    let Some(expected) = infocmp(database.path(), "xterm-ghostty") else {
        return;
    };
    assert_eq!(ours(&database.path().join("x/xterm-ghostty")), expected);
}

#[test]
#[cfg(target_os = "linux")]
fn the_system_databases_entries_decompile_to_what_infocmp_prints() {
    let databases = ["/usr/share/terminfo", "/lib/terminfo", "/etc/terminfo"].map(Path::new);
    let names = [
        "xterm-ghostty",
        "xterm-256color",
        "xterm",
        "screen-256color",
        "tmux-256color",
        "vt100",
        "linux",
        "dumb",
        "rxvt-unicode-256color",
        "xterm-kitty",
        "alacritty",
        "foot",
        "wezterm",
    ];
    let mut compared = 0;
    for name in names {
        let first = &name[..1];
        let Some(database) = databases
            .iter()
            .find(|database| database.join(first).join(name).is_file())
        else {
            continue;
        };
        let Some(expected) = infocmp(database, name) else {
            return;
        };
        assert_eq!(ours(&database.join(first).join(name)), expected, "{name}");
        compared += 1;
    }
    assert!(
        compared > 0 || !Path::new("/usr/bin/infocmp").exists(),
        "no entry to compare"
    );
}
