//! The app's handle on a shown pane ([`Pane`]) and the input it takes, as `UniFFI` records over `ngterm-term`'s own
//! input types: a key is `ngterm-term`'s `KeyInput` (its key named as Ghostty names the physical key), a pointer event
//! its `MouseInput`, a wheel turn its `WheelInput`. The handle calls the pane's terminal directly, on the caller's
//! thread, so typing never waits on the actor; what needs the actor (taking the lead, hiding, clipboard answers) is on
//! [`crate::MobileCore`].
//!
//! The terminal behind a handle changes when the pane is attached again (after a lost link, a respawn, a takeover):
//! the handle always reaches the current one, and calls with none (a pane hidden) fail with
//! [`PaneInputError::Closed`].

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ngterm_core::ghostty_config::binds::{KeyAction, Mods, PhysicalKey};
use ngterm_term::{
    ClipboardLocation, GridPoint, GridSize, KeyInput, ModSides, MouseAction, MouseButton, MouseInput, Paste,
    PasteSource, SelectionText, Wake, Wheel, WheelInput,
};

/// A pane's grid: cells and the pixel size of one cell (`ngterm-term`'s `GridSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PaneSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u32,
    pub cell_height: u32,
}

impl From<PaneSize> for GridSize {
    fn from(size: PaneSize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

impl From<GridSize> for PaneSize {
    fn from(size: GridSize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

/// A key's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneKeyAction {
    Press,
    Repeat,
    Release,
}

impl From<PaneKeyAction> for KeyAction {
    fn from(action: PaneKeyAction) -> Self {
        match action {
            PaneKeyAction::Press => Self::Press,
            PaneKeyAction::Repeat => Self::Repeat,
            PaneKeyAction::Release => Self::Release,
        }
    }
}

/// A key event (`ngterm-term`'s `KeyInput`). Modifiers are bits: shift 1, ctrl 2, alt 4, super 8, caps lock 16, num
/// lock 32.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaneKey {
    pub action: PaneKeyAction,
    /// Ghostty's name of the key's position, whatever the layout (`key_a`, `enter`, `arrow_up`, `digit_1`, …); a
    /// name it doesn't know is `unidentified`.
    pub key: String,
    pub mods: u8,
    /// The modifiers the layout used up producing `text` (shift for `A`).
    pub consumed: u8,
    /// Part of an input-method composition (a dead key, a preedit).
    pub composing: bool,
    /// What the key types under the layout, before ctrl and alt; empty for none.
    pub text: String,
    /// The character the key types with no modifier (the Kitty protocol's key code); `None` guesses it from a
    /// one-character `text`.
    pub unshifted: Option<String>,
}

/// A pointer event's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneMouseAction {
    Press,
    Release,
    Motion,
}

/// A mouse button (a touch is the left one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneMouseButton {
    Left,
    Right,
    Middle,
}

/// A pointer event (`ngterm-term`'s `MouseInput`), in pixels from the pane's top left.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PaneMouse {
    pub action: PaneMouseAction,
    /// `None` for a motion with no button.
    pub button: Option<PaneMouseButton>,
    pub mods: u8,
    pub x: f32,
    pub y: f32,
}

/// A wheel or scroll gesture in whole steps (`ngterm-term`'s `WheelInput`): positive is up (rows) and right
/// (columns).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PaneWheel {
    pub rows: i32,
    pub columns: i32,
    pub mods: u8,
    pub x: f32,
    pub y: f32,
}

/// Where a wheel turn went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneWheelOutcome {
    /// Reported to the program as wheel buttons.
    Reported,
    /// Sent to the program as cursor keys (alternate scroll).
    CursorKeys,
    /// The view scrolled back or forward.
    Scrolled,
    Nothing,
}

/// What a paste did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneSendOutcome {
    Written,
    Empty,
    /// The text could run a command: nothing was written. Confirm with the user and paste again allowing it.
    Unsafe,
}

/// A cell of the visible rows: column `x`, row `y`, from the top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CellPoint {
    pub x: u16,
    pub y: u32,
}

impl From<CellPoint> for GridPoint {
    fn from(point: CellPoint) -> Self {
        Self::viewport(point.x, point.y)
    }
}

/// Why a pane refused input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PaneInputError {
    /// The pane has no terminal (hidden) or its channel is gone.
    #[error("the pane is closed")]
    Closed,
    /// The program is not reading its input; try again later.
    #[error("the pane's input queue is full")]
    Backpressure,
    /// The terminal refused the input (a point outside the grid, a zero cell size).
    #[error("{reason}")]
    Refused { reason: String },
}

impl From<ngterm_term::PaneError> for PaneInputError {
    fn from(error: ngterm_term::PaneError) -> Self {
        match error {
            ngterm_term::PaneError::Backpressure => Self::Backpressure,
            ngterm_term::PaneError::Closed => Self::Closed,
            other => Self::Refused {
                reason: other.to_string(),
            },
        }
    }
}

/// Why [`Pane::resize`] refused a grid with no columns or rows.
const EMPTY_GRID: &str = "a pane needs at least one column and one row";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a [`Pane`] handle and the actor share: the pane's current terminal and its grid.
pub(crate) struct PaneSlot {
    terminal: Mutex<Option<Arc<ngterm_term::Pane>>>,
    size: Mutex<GridSize>,
    /// Tells the actor the view gave a grid while the slot held no terminal: a pane waiting for one attaches.
    sized: Wake,
}

impl std::fmt::Debug for PaneSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PaneSlot")
            .field("terminal", &self.terminal)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl PaneSlot {
    /// A slot at `size`; `sized` runs when a grid is given with no terminal in place.
    pub(crate) fn new(size: GridSize, sized: Wake) -> Self {
        Self {
            terminal: Mutex::new(None),
            size: Mutex::new(size),
            sized,
        }
    }

    /// The current terminal, if any.
    pub(crate) fn terminal(&self) -> Option<Arc<ngterm_term::Pane>> {
        lock(&self.terminal).clone()
    }

    /// Puts `terminal` in place of the current one.
    pub(crate) fn replace(&self, terminal: Option<Arc<ngterm_term::Pane>>) {
        *lock(&self.terminal) = terminal;
    }

    /// Puts `terminal`, built at `built`, in place of the current one, and sizes it to the grid the view gave since:
    /// a [`Pane::resize`] that raced the build stored its size and resized the terminal this one replaces.
    pub(crate) fn install(&self, terminal: Arc<ngterm_term::Pane>, built: GridSize) {
        let mut current = lock(&self.terminal);
        let size = self.size();
        if size != built {
            // the stored size passed the zero check, so the terminal takes it
            let _ = terminal.resize(size);
        }
        *current = Some(terminal);
    }

    /// The grid the view last gave the pane.
    pub(crate) fn size(&self) -> GridSize {
        *lock(&self.size)
    }

    /// Stores the grid the view gave.
    pub(crate) fn set_size(&self, size: GridSize) {
        *lock(&self.size) = size;
    }

    fn current(&self) -> Result<Arc<ngterm_term::Pane>, PaneInputError> {
        self.terminal().ok_or(PaneInputError::Closed)
    }
}

/// A shown pane of an attached session ([`crate::MobileCore::show_pane`]): input goes to its terminal, which the core
/// feeds from the pane's ssh channel.
#[derive(Debug, uniffi::Object)]
pub struct Pane {
    id: String,
    slot: Arc<PaneSlot>,
}

impl Pane {
    pub(crate) fn new(id: String, slot: Arc<PaneSlot>) -> Self {
        Self { id, slot }
    }

    /// The terminal behind the handle now (the tests read its screen).
    #[cfg(test)]
    pub(crate) fn terminal(&self) -> Option<Arc<ngterm_term::Pane>> {
        self.slot.terminal()
    }
}

fn mods(bits: u8) -> Mods {
    Mods::from_bits_truncate(bits)
}

#[uniffi::export]
impl Pane {
    /// The pane's identity in its session, as the session's layout names it.
    #[must_use]
    pub fn id(&self) -> String {
        self.id.clone()
    }

    /// The grid the view last gave the pane.
    #[must_use]
    pub fn size(&self) -> PaneSize {
        self.slot.size().into()
    }

    /// The view's new grid: the terminal reflows and the machine's pty follows (only the pane's leader sizes it). A
    /// pane shown with no grid attaches once the view gives it one.
    ///
    /// # Errors
    ///
    /// [`PaneInputError::Refused`] for a size the terminal can't take. A size with no columns or rows is refused
    /// and not kept; any other is kept for the next attach anyway.
    pub fn resize(&self, size: PaneSize) -> Result<(), PaneInputError> {
        if size.cols == 0 || size.rows == 0 {
            return Err(PaneInputError::Refused {
                reason: EMPTY_GRID.to_owned(),
            });
        }
        self.slot.set_size(size.into());
        if let Some(terminal) = self.slot.terminal() {
            return Ok(terminal.resize(size.into())?);
        }
        (self.slot.sized)();
        Ok(())
    }

    /// Sends a key, encoded for the program's keyboard modes; whether anything was sent (a lone modifier sends
    /// nothing).
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    #[expect(
        clippy::needless_pass_by_value,
        reason = "UniFFI passes the foreign side's arguments owned"
    )]
    pub fn key(&self, key: PaneKey) -> Result<bool, PaneInputError> {
        let terminal = self.slot.current()?;
        let mut input = KeyInput::press(
            PhysicalKey::new(&key.key).unwrap_or(PhysicalKey::UNIDENTIFIED),
            mods(key.mods),
            &key.text,
        );
        input.action = key.action.into();
        input.consumed = mods(key.consumed);
        input.sides = ModSides::empty();
        input.composing = key.composing;
        if let Some(unshifted) = key.unshifted.as_deref().and_then(|text| text.chars().next()) {
            input.unshifted = Some(unshifted);
        }
        Ok(terminal.key(&input)?)
    }

    /// Types `text` as the keyboard would (an input method's commit, the extra-keys row); whether anything was sent.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    #[expect(
        clippy::needless_pass_by_value,
        reason = "UniFFI passes the foreign side's arguments owned"
    )]
    pub fn type_text(&self, text: String) -> Result<bool, PaneInputError> {
        Ok(self.slot.current()?.type_text(&text)?)
    }

    /// Reports a pointer event to the program when it asked for the mouse ([`Pane::mouse_tracking`]); whether
    /// anything was reported.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn mouse(&self, mouse: PaneMouse) -> Result<bool, PaneInputError> {
        let input = MouseInput {
            action: match mouse.action {
                PaneMouseAction::Press => MouseAction::Press,
                PaneMouseAction::Release => MouseAction::Release,
                PaneMouseAction::Motion => MouseAction::Motion,
            },
            button: mouse.button.map(|button| match button {
                PaneMouseButton::Left => MouseButton::Left,
                PaneMouseButton::Right => MouseButton::Right,
                PaneMouseButton::Middle => MouseButton::Middle,
            }),
            mods: mods(mouse.mods),
            x: mouse.x,
            y: mouse.y,
        };
        Ok(self.slot.current()?.mouse(&input)?)
    }

    /// Whether the program takes pointer events: while it does, a drag is the program's, not a selection.
    #[must_use]
    pub fn mouse_tracking(&self) -> bool {
        self.slot.terminal().is_some_and(|terminal| terminal.mouse_tracking())
    }

    /// Routes a wheel turn or scroll gesture: to the program as buttons or cursor keys, or scrolling the view.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn wheel(&self, wheel: PaneWheel) -> Result<PaneWheelOutcome, PaneInputError> {
        let input = WheelInput {
            rows: isize::try_from(wheel.rows).unwrap_or_default(),
            columns: isize::try_from(wheel.columns).unwrap_or_default(),
            mods: mods(wheel.mods),
            x: wheel.x,
            y: wheel.y,
        };
        Ok(match self.slot.current()?.wheel(&input)? {
            Wheel::Reported => PaneWheelOutcome::Reported,
            Wheel::CursorKeys => PaneWheelOutcome::CursorKeys,
            Wheel::Scrolled => PaneWheelOutcome::Scrolled,
            Wheel::Nothing => PaneWheelOutcome::Nothing,
        })
    }

    /// Pastes `text` from the clipboard, framed for the program's modes; text that could run a command is refused
    /// ([`PaneSendOutcome::Unsafe`]) unless `allow_unsafe`.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    #[expect(
        clippy::needless_pass_by_value,
        reason = "UniFFI passes the foreign side's arguments owned"
    )]
    pub fn paste(&self, text: String, allow_unsafe: bool) -> Result<PaneSendOutcome, PaneInputError> {
        let terminal = self.slot.current()?;
        let pasted = terminal.paste(&text, PasteSource::Clipboard, ClipboardLocation::Standard, allow_unsafe)?;
        Ok(match pasted {
            Paste::Written => PaneSendOutcome::Written,
            Paste::Empty => PaneSendOutcome::Empty,
            Paste::Unsafe => PaneSendOutcome::Unsafe,
        })
    }

    /// The pane gained or lost focus (a program that asked hears of it); whether a report was sent.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn focus(&self, focused: bool) -> Result<bool, PaneInputError> {
        Ok(self.slot.current()?.focus(focused)?)
    }

    /// Selects from `start` to `end` (either order), as a run of text or a `rectangle`.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn select(&self, start: CellPoint, end: CellPoint, rectangle: bool) -> Result<(), PaneInputError> {
        Ok(self.slot.current()?.select(start.into(), end.into(), rectangle)?)
    }

    /// Selects the word at `at`; false where there is none.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn select_word(&self, at: CellPoint) -> Result<bool, PaneInputError> {
        Ok(self.slot.current()?.select_word(at.into())?)
    }

    /// Selects the line at `at`, across soft wraps; false on a blank line.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn select_line(&self, at: CellPoint) -> Result<bool, PaneInputError> {
        Ok(self.slot.current()?.select_line(at.into())?)
    }

    /// Selects everything; false on a blank screen.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn select_all(&self) -> Result<bool, PaneInputError> {
        Ok(self.slot.current()?.select_all()?)
    }

    /// Drops the selection.
    ///
    /// # Errors
    ///
    /// [`PaneInputError`].
    pub fn clear_selection(&self) -> Result<(), PaneInputError> {
        Ok(self.slot.current()?.clear_selection()?)
    }

    /// The selection's text as a copy takes it (each line's trailing blanks trimmed); `None` with none.
    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        self.slot.terminal()?.selection_text(SelectionText::Copy)
    }
}
