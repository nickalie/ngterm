//! A floating overlay sized in points per axis (upstream's `OverlayPointSize.swift`): `session.overlay.open|resize
//! --width/--height` override the panel's `--size-percent` on the axes they give, in points of the area the program or
//! page gets, a page's strip coming on top. [`panel_size`] is the one sizing function: the frontend lays the panel out
//! with it and the tree's `overlayWidth`/`overlayHeight` read back through it from the session's cached detail frame,
//! so the two cannot disagree. Ephemeral, like the slot: never persisted.

use uuid::Uuid;

use super::session::Session;
use super::store::AppStore;
use crate::html::HtmlOverlay;
use crate::hud;

/// `--width`/`--height` of a floating program or page, in points of its content area (upstream's `OverlayPointSize`).
/// An axis left `None` keeps the percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlayPointSize {
    pub width: Option<i64>,
    pub height: Option<i64>,
}

impl OverlayPointSize {
    /// Neither axis is given.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width.is_none() && self.height.is_none()
    }

    /// These sizes with every axis `other` gives replaced (upstream's `merging`).
    #[must_use]
    pub fn merging(self, other: Self) -> Self {
        Self {
            width: other.width.or(self.width),
            height: other.height.or(self.height),
        }
    }
}

/// The most of the session a point axis may take, the HUD's cap (upstream's `maxShare`), so a panel sized in points
/// always leaves some of the session in view. A percent axis keeps its 1...100.
#[expect(clippy::cast_precision_loss, reason = "a percent within 0...100")]
pub const MAX_SHARE: f64 = hud::MAX_SIZE_PERCENT as f64 / 100.0;

/// The floating panel's outer size inside `bounds` (width, height), `chrome` being the strip a page draws above its
/// content (upstream's `panelSize`). A point axis is capped AFTER the chrome is added, so the strip cannot push the
/// panel past [`MAX_SHARE`].
#[must_use]
pub fn panel_size(bounds: (f64, f64), size_percent: i64, points: OverlayPointSize, chrome: f64) -> (f64, f64) {
    let share = percent_share(size_percent);
    let (width, height) = bounds;
    let panel_width = points
        .width
        .map_or(width * share, |points| point_value(points).min(width * MAX_SHARE));
    let panel_height = points.height.map_or(height * share, |points| {
        (point_value(points) + chrome).min(height * MAX_SHARE)
    });
    (panel_width, panel_height)
}

/// What [`panel_size`] leaves the program or page once the chrome is taken, never below zero (upstream's
/// `contentSize`).
#[must_use]
pub fn content_size(bounds: (f64, f64), size_percent: i64, points: OverlayPointSize, chrome: f64) -> (f64, f64) {
    let (width, height) = panel_size(bounds, size_percent, points, chrome);
    (width, (height - chrome).max(0.0))
}

/// A percent as a share of the session.
fn percent_share(percent: i64) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "a percent within 1...100 by the slot's clamp")]
    let percent = percent.clamp(1, 100) as f64;
    percent / 100.0
}

/// A point size as a float; far past any screen, so the precision lost above 2^53 never shows.
fn point_value(points: i64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a caller's point size, capped at a share of the session"
    )]
    let points = points as f64;
    points
}

/// What a page opening asks of the session-wide slot's size: a floating percent (`None` covers the session) and the
/// point axes overriding it, ignored without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlaySize {
    pub percent: Option<i64>,
    pub points: OverlayPointSize,
}

impl OverlaySize {
    /// A size by its percent alone.
    #[must_use]
    pub fn percent(percent: Option<i64>) -> Self {
        Self {
            percent,
            points: OverlayPointSize::default(),
        }
    }
}

/// How a floating program or page is sized: its percent, the point axes overriding it and its chrome (upstream's
/// `OverlayPanelStyle.PointSizing`). The frontend lays the panel out with [`PanelSizing::panel_size`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelSizing {
    pub size_percent: i64,
    pub points: OverlayPointSize,
    pub chrome: f64,
}

impl PanelSizing {
    /// A panel sized by its percent alone.
    #[must_use]
    pub fn percent(size_percent: i64) -> Self {
        Self {
            size_percent,
            points: OverlayPointSize::default(),
            chrome: 0.0,
        }
    }

    /// The panel's outer size inside `bounds` (width, height).
    #[must_use]
    pub fn panel_size(self, bounds: (f64, f64)) -> (f64, f64) {
        panel_size(bounds, self.size_percent, self.points, self.chrome)
    }
}

impl super::overlay::OverlaySlot {
    /// How the occupant floats: its percent, its point axes and a page's chrome; `None` for a full-pane cover. A HUD
    /// carries no points, so its sizing is its percent alone.
    #[must_use]
    pub fn panel_sizing(&self) -> Option<PanelSizing> {
        Some(PanelSizing {
            size_percent: self.size_percent?,
            points: self.points,
            chrome: self.page().map_or(0.0, HtmlOverlay::chrome),
        })
    }
}

impl Session {
    /// A floating program or page's content size in points (width, height) in the cached detail frame: the read-back
    /// of `--width`/`--height` and of `--size-percent` alike (upstream's `overlayContentSize`). `None` for a full
    /// overlay, a HUD, an empty slot, and before the deck has laid the session out.
    #[must_use]
    pub fn overlay_content_size(&self) -> Option<(f64, f64)> {
        if !self.cover_overlay_active() {
            return None;
        }
        let sizing = self.overlay.panel_sizing()?;
        let detail = self
            .hud_frames
            .detail
            .filter(|frame| frame.width > 0.0 && frame.height > 0.0)?;
        Some(content_size(
            (detail.width, detail.height),
            sizing.size_percent,
            sizing.points,
            sizing.chrome,
        ))
    }
}

/// Why [`AppStore::resize_overlay_in_points`] refused point sizes (upstream's `OverlayPointResizeFailure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPointResizeFailure {
    NoOverlay,
    /// The slot is shown on another machine, or this one shows another machine's overlay.
    Remote,
    /// A HUD measures its own height.
    Hud,
    /// Points alone need a floating panel whose percent the other axis keeps.
    NotFloating,
}

/// Upstream's `OverlayPointError.notFloating`.
pub const POINTS_NOT_FLOATING: &str =
    "session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too";
/// Upstream's `OverlayPointError.hud`.
pub const POINTS_HUD: &str = "a hud measures its own height: --width/--height do not apply";
/// Upstream's `OverlayPointError.remote`: the presentation wire carries only a percent.
pub const POINTS_REMOTE: &str = "--width/--height are not supported for an overlay shown on another Mac";

impl OverlayPointResizeFailure {
    /// The control error for this refusal (upstream's `message`).
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoOverlay => "no overlay",
            Self::Remote => POINTS_REMOTE,
            Self::Hud => POINTS_HUD,
            Self::NotFloating => POINTS_NOT_FLOATING,
        }
    }
}

impl AppStore {
    /// Resizes the session-wide occupant with point sizes (upstream's `resizeOverlayInPoints`): with a percent, the
    /// panel re-sizes from it keeping exactly `points`; without one, only the given axes of a panel already floating
    /// change. Refused for a slot shown on, or replicated from, another machine (a program replica is the core's to
    /// know, and it refuses before asking), and for a HUD.
    ///
    /// # Errors
    ///
    /// The refusal, nothing changed.
    pub fn resize_overlay_in_points(
        &mut self,
        id: Uuid,
        size_percent: Option<i64>,
        points: OverlayPointSize,
    ) -> Result<(), OverlayPointResizeFailure> {
        let session = self.session(id).ok_or(OverlayPointResizeFailure::NoOverlay)?;
        let replica = session.overlay.page().is_some_and(|page| page.replica_of.is_some());
        if session.remote_overlays().slot(None).is_some() || replica {
            return Err(OverlayPointResizeFailure::Remote);
        }
        if session.overlay.hud_active() {
            return Err(OverlayPointResizeFailure::Hud);
        }
        if !session.cover_overlay_active() {
            return Err(OverlayPointResizeFailure::NoOverlay);
        }
        if let Some(percent) = size_percent {
            self.resize_overlay_with_points(id, Some(percent), points);
            return Ok(());
        }
        let slot = self
            .session_mut(id)
            .map(|session| &mut session.overlay)
            .ok_or(OverlayPointResizeFailure::NoOverlay)?;
        if slot.size_percent.is_none() {
            return Err(OverlayPointResizeFailure::NotFloating);
        }
        slot.points = slot.points.merging(points);
        Ok(())
    }
}
