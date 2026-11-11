//! Typed results of `terminal.h`'s `ghostty_terminal_vt_write_until` at `LIBGHOSTTY_VT_REV`.

use crate::ffi;

bitflags::bitflags! {
    /// Parser boundaries to stop at. Ground is a state: asking for it while there consumes zero bytes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Boundaries: u32 {
        const GROUND = ffi::GHOSTTY_TERMINAL_BOUNDARY_GROUND as u32;
        const APC_START = ffi::GHOSTTY_TERMINAL_BOUNDARY_APC_START as u32;
        const APC_END = ffi::GHOSTTY_TERMINAL_BOUNDARY_APC_END as u32;
        const DCS_START = ffi::GHOSTTY_TERMINAL_BOUNDARY_DCS_START as u32;
        const DCS_END = ffi::GHOSTTY_TERMINAL_BOUNDARY_DCS_END as u32;
        const OSC_START = ffi::GHOSTTY_TERMINAL_BOUNDARY_OSC_START as u32;
        const OSC_END = ffi::GHOSTTY_TERMINAL_BOUNDARY_OSC_END as u32;
    }
}

/// The string that ended, or (for a start alone) the string that started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    /// APC, SOS and PM share the same parser state.
    Apc,
    Dcs,
    Osc,
}

/// How the string started. C1 introducers are recognized only outside ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringIntroducer {
    Esc,
    C1,
}

/// How a string ended. Only `StEsc` and `StC1` dispatch Kitty graphics commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringEnd {
    /// The ESC of ST, consumed; its backslash remains for the next write.
    StEsc,
    StC1,
    Cancelled,
    Aborted,
    Bel,
}

/// The consumed prefix and requested boundaries reached by its last byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteStop {
    pub consumed: usize,
    /// Empty on exhaustion. Includes only requested flags; an end and a different start can coincide.
    pub reached: Boundaries,
    pub kind: Option<StringKind>,
    /// Set only for starts, including a start that also aborted another string.
    pub introducer: Option<StringIntroducer>,
    pub end: Option<StringEnd>,
    /// Set only for string starts or ends.
    pub byte: Option<u8>,
}

impl WriteStop {
    pub(crate) fn from_raw(raw: ffi::GhosttyTerminalWriteStop) -> Self {
        let reached = Boundaries::from_bits_truncate(raw.reached);
        let starts = reached.intersects(Boundaries::APC_START | Boundaries::OSC_START | Boundaries::DCS_START);
        let ends = reached.intersects(Boundaries::APC_END | Boundaries::OSC_END | Boundaries::DCS_END);
        Self {
            consumed: raw.consumed,
            reached,
            kind: (starts || ends).then_some(match raw.kind {
                ffi::GHOSTTY_TERMINAL_STRING_KIND_DCS => StringKind::Dcs,
                ffi::GHOSTTY_TERMINAL_STRING_KIND_OSC => StringKind::Osc,
                _ => StringKind::Apc,
            }),
            introducer: starts.then_some(match raw.introducer {
                ffi::GHOSTTY_TERMINAL_STRING_INTRODUCER_C1 => StringIntroducer::C1,
                _ => StringIntroducer::Esc,
            }),
            end: ends.then_some(match raw.end {
                ffi::GHOSTTY_TERMINAL_STRING_END_ST_C1 => StringEnd::StC1,
                ffi::GHOSTTY_TERMINAL_STRING_END_CANCELLED => StringEnd::Cancelled,
                ffi::GHOSTTY_TERMINAL_STRING_END_ABORTED => StringEnd::Aborted,
                ffi::GHOSTTY_TERMINAL_STRING_END_BEL => StringEnd::Bel,
                _ => StringEnd::StEsc,
            }),
            byte: (starts || ends).then_some(raw.byte),
        }
    }
}
