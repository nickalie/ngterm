//! Sidebar row tints (upstream's `SidebarTint.swift`, `ControlTintUpdate` in `ControlModes.swift` and
//! `AppStore+Tint.swift`): one color from a fixed palette, shown on a session or workspace row's icon, its name, or
//! both. A workspace's tint colors its own row only; the sessions inside keep theirs.
//!
//! The store seam here is what `session.tint`, `workspace.tint`, the row menus' Tint submenu and the palette's Tint
//! Session rows share: an unchanged value is a no-op, so a repeated set neither saves nor emits `tree.changed`.

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use super::store::AppStore;

/// The fixed palette a tint picks from. The wire and snapshot spelling is the lower-case name; the frontend maps each
/// to the macOS system color of the same name, light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TintColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl TintColor {
    /// Every color, in upstream's declaration order (the menus', the palette's and the accepted-names list's).
    pub const ALL: [Self; 7] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Gray,
    ];

    /// The wire, snapshot and read-back spelling.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Gray => "gray",
        }
    }

    /// The name a menu or palette row shows (upstream's `rawValue.capitalized`).
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Gray => "Gray",
        }
    }

    /// Parses the wire spelling exactly; `None` for anything else.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.wire_name() == name)
    }

    /// The accepted names pipe-joined, for help and rejection text (upstream's `validNamesList`).
    #[must_use]
    pub fn valid_names_list() -> String {
        Self::ALL.map(Self::wire_name).join("|")
    }
}

/// A part of a sidebar row a tint can color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TintPart {
    Icon,
    Name,
}

impl TintPart {
    /// Every part, in upstream's order: a tint's parts are kept in it.
    pub const ALL: [Self; 2] = [Self::Icon, Self::Name];

    /// The wire and snapshot spelling.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Name => "name",
        }
    }

    /// Parses the wire spelling exactly; `None` for anything else.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.wire_name() == name)
    }

    /// The accepted names comma-joined, for help and rejection text (upstream's `validNamesList`).
    #[must_use]
    pub fn valid_names_list() -> String {
        Self::ALL.map(Self::wire_name).join(",")
    }
}

/// A set of row parts; iterates in [`TintPart::ALL`] order, so two sets listing the same parts are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TintParts {
    icon: bool,
    name: bool,
}

impl TintParts {
    pub const ICON: Self = Self {
        icon: true,
        name: false,
    };
    pub const NAME: Self = Self {
        icon: false,
        name: true,
    };
    pub const BOTH: Self = Self { icon: true, name: true };

    /// The set of `parts`, repeats collapsing.
    #[must_use]
    pub fn of(parts: impl IntoIterator<Item = TintPart>) -> Self {
        parts.into_iter().fold(Self::default(), |set, part| match part {
            TintPart::Icon => Self { icon: true, ..set },
            TintPart::Name => Self { name: true, ..set },
        })
    }

    #[must_use]
    pub fn contains(self, part: TintPart) -> bool {
        match part {
            TintPart::Icon => self.icon,
            TintPart::Name => self.name,
        }
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.icon && !self.name
    }

    /// The parts in [`TintPart::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = TintPart> {
        TintPart::ALL.into_iter().filter(move |part| self.contains(*part))
    }

    /// The parts' wire names in order, as `tree` reads them back (`tintOn`).
    #[must_use]
    pub fn wire_names(self) -> Vec<String> {
        self.iter().map(|part| part.wire_name().to_owned()).collect()
    }

    /// Reads a wire placement list, each entry trimmed and lower-cased (upstream's `SidebarTint.parseParts`); `None`
    /// when it is empty or names an unknown part.
    #[must_use]
    pub fn parse(raw: &[String]) -> Option<Self> {
        if raw.is_empty() {
            return None;
        }
        raw.iter()
            .map(|entry| TintPart::from_wire(&entry.trim().to_lowercase()))
            .collect::<Option<Vec<_>>>()
            .map(Self::of)
    }
}

/// A session or workspace row's tint: one color on one or more parts. Its parts are never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SidebarTint {
    color: TintColor,
    parts: TintParts,
}

impl SidebarTint {
    /// A tint of `color` on `parts`, the icon when `parts` is empty: the one placement every tint must have.
    #[must_use]
    pub fn new(color: TintColor, parts: TintParts) -> Self {
        let parts = if parts.is_empty() { TintParts::ICON } else { parts };
        Self { color, parts }
    }

    /// A tint of `color` on the icon.
    #[must_use]
    pub fn on_icon(color: TintColor) -> Self {
        Self::new(color, TintParts::ICON)
    }

    #[must_use]
    pub fn color(self) -> TintColor {
        self.color
    }

    #[must_use]
    pub fn parts(self) -> TintParts {
        self.parts
    }

    /// Whether the tint colors `part` of the row.
    #[must_use]
    pub fn shows(self, part: TintPart) -> bool {
        self.parts.contains(part)
    }

    /// The tint a row with `current` gets for `color`: its own parts kept, an untinted row starting on the icon. `None`
    /// clears.
    #[must_use]
    pub fn recolor(current: Option<Self>, color: Option<TintColor>) -> Option<Self> {
        color.map(|color| Self::new(color, current.map_or(TintParts::ICON, Self::parts)))
    }

    /// `current`'s color shown on `parts` instead. An untinted row stays untinted, so no row holds a placement it does
    /// not show.
    #[must_use]
    pub fn place(current: Option<Self>, parts: TintParts) -> Option<Self> {
        current.map(|tint| Self::new(tint.color, parts))
    }
}

/// Persisted as `{"color": "red", "on": ["icon"]}`. Decoding is lossy like the snapshots around it: an unknown color
/// fails, so the caller's lossy field drops the tint alone, while unknown parts are skipped and an empty, missing or
/// malformed list reads as the icon.
impl Serialize for SidebarTint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("SidebarTint", 2)?;
        state.serialize_field("color", self.color.wire_name())?;
        state.serialize_field("on", &self.parts.wire_names())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SidebarTint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Stored {
            color: String,
            #[serde(default)]
            on: serde_json::Value,
        }
        let stored = Stored::deserialize(deserializer)?;
        let color = TintColor::from_wire(&stored.color)
            .ok_or_else(|| D::Error::custom(format!("unknown tint color {}", stored.color)))?;
        let names: Vec<String> = serde_json::from_value(stored.on).unwrap_or_default();
        let parts = TintParts::of(names.iter().filter_map(|name| TintPart::from_wire(name)));
        Ok(Self::new(color, parts))
    }
}

/// A parsed `session.tint`/`workspace.tint` request (upstream's `ControlTintUpdate`): a color with optional parts, or
/// a clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintUpdate {
    Set(TintColor, Option<TintParts>),
    Clear,
}

impl TintUpdate {
    /// The tint this update leaves on a row that has `current`: a set without parts keeps the row's own, starting at
    /// the icon.
    #[must_use]
    pub fn resolved(self, current: Option<SidebarTint>) -> Option<SidebarTint> {
        match self {
            Self::Clear => None,
            Self::Set(color, Some(parts)) => Some(SidebarTint::new(color, parts)),
            Self::Set(color, None) => SidebarTint::recolor(current, Some(color)),
        }
    }
}

/// One choice of the row menu's Tint submenu (upstream's `TintMenuRequest.Choice`): a color (`None` is None), or where
/// the tinted rows show their own colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintChoice {
    Color(Option<TintColor>),
    Parts(TintParts),
}

impl TintChoice {
    /// The tint a row with `current` gets for this choice.
    #[must_use]
    pub fn applied(self, current: Option<SidebarTint>) -> Option<SidebarTint> {
        match self {
            Self::Color(color) => SidebarTint::recolor(current, color),
            Self::Parts(parts) => SidebarTint::place(current, parts),
        }
    }
}

impl AppStore {
    /// Replaces a session's tint (`None` clears) and persists; returns whether it changed. An unknown id or an
    /// unchanged value is a no-op.
    pub fn set_session_tint(&mut self, tint: Option<SidebarTint>, id: Uuid) -> bool {
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        if session.tint == tint {
            return false;
        }
        session.tint = tint;
        self.schedule_tree_changed();
        self.save();
        true
    }

    /// Replaces a workspace row's own tint (`None` clears) and persists, with the session form's no-op rule.
    pub fn set_workspace_tint(&mut self, tint: Option<SidebarTint>, id: Uuid) -> bool {
        let Some(index) = self.workspace_index(id) else {
            return false;
        };
        if self.workspaces[index].tint == tint {
            return false;
        }
        self.workspaces[index].tint = tint;
        self.schedule_tree_changed();
        self.save();
        true
    }

    /// Applies a Tint submenu choice to every session in `ids`: a color keeps each tinted row's parts and starts an
    /// untinted one on the icon, a placement moves each tinted row's own color and leaves an untinted one alone. One
    /// save for the batch; returns whether any row changed.
    pub fn apply_session_tints(&mut self, choice: TintChoice, ids: &[Uuid]) -> bool {
        let mut changed = false;
        for session in self
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
            .filter(|session| ids.contains(&session.id()))
        {
            let tint = choice.applied(session.tint);
            if tint != session.tint {
                session.tint = tint;
                changed = true;
            }
        }
        if changed {
            self.schedule_tree_changed();
            self.save();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// `partsAreOrderedAndNeverEmpty`.
    #[test]
    fn parts_are_ordered_and_never_empty() {
        let both = SidebarTint::new(TintColor::Red, TintParts::of([TintPart::Name, TintPart::Icon]));
        assert_eq!(
            both.parts().iter().collect::<Vec<_>>(),
            [TintPart::Icon, TintPart::Name]
        );
        assert_eq!(
            SidebarTint::new(TintColor::Red, TintParts::default()).parts(),
            TintParts::ICON
        );
        assert_eq!(both, SidebarTint::new(TintColor::Red, TintParts::BOTH));
        let named = SidebarTint::new(TintColor::Red, TintParts::NAME);
        assert!(named.shows(TintPart::Name));
        assert!(!named.shows(TintPart::Icon));
        assert_eq!(
            TintColor::valid_names_list(),
            "red|orange|yellow|green|blue|purple|gray"
        );
        assert_eq!(TintPart::valid_names_list(), "icon,name");
    }

    /// `parsePartsAcceptsKnownParts`, `parsePartsRejectsEmptyOrUnknown`.
    #[test]
    fn parse_accepts_known_parts_and_rejects_empty_or_unknown() {
        for (raw, parts) in [
            (strings(&["icon"]), TintParts::ICON),
            (strings(&["name", "icon"]), TintParts::BOTH),
            (strings(&[" Name "]), TintParts::NAME),
            (strings(&["icon", "icon"]), TintParts::ICON),
        ] {
            assert_eq!(TintParts::parse(&raw), Some(parts), "{raw:?}");
        }
        for raw in [
            strings(&[]),
            strings(&["strip"]),
            strings(&["icon", ""]),
            strings(&["icon", "both"]),
        ] {
            assert_eq!(TintParts::parse(&raw), None, "{raw:?}");
        }
    }

    /// `recolorKeepsPartsAndStartsUntintedOnTheIcon`, `placeKeepsColorAndLeavesUntintedAlone`.
    #[test]
    fn recolor_keeps_the_parts_and_place_keeps_the_color() {
        let named = SidebarTint::new(TintColor::Green, TintParts::NAME);
        assert_eq!(
            SidebarTint::recolor(Some(named), Some(TintColor::Red)),
            Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
        );
        assert_eq!(
            SidebarTint::recolor(None, Some(TintColor::Red)),
            Some(SidebarTint::on_icon(TintColor::Red))
        );
        assert_eq!(SidebarTint::recolor(Some(named), None), None);
        assert_eq!(
            SidebarTint::place(Some(SidebarTint::on_icon(TintColor::Blue)), TintParts::NAME),
            Some(SidebarTint::new(TintColor::Blue, TintParts::NAME))
        );
        assert_eq!(SidebarTint::place(None, TintParts::NAME), None);
        assert_eq!(TintChoice::Color(None).applied(Some(named)), None);
        assert_eq!(TintChoice::Parts(TintParts::BOTH).applied(None), None);
    }

    /// `codableRoundTripsColorAndParts`.
    #[test]
    fn the_snapshot_form_round_trips_color_and_parts() {
        let tint = SidebarTint::new(TintColor::Purple, TintParts::BOTH);
        let text = serde_json::to_string(&tint).unwrap();
        assert_eq!(text, r#"{"color":"purple","on":["icon","name"]}"#);
        assert_eq!(serde_json::from_str::<SidebarTint>(&text).unwrap(), tint);
    }

    /// `decodeSkipsUnknownPartsAndDefaultsToTheIcon`, `decodeThrowsOnAnUnknownColor`.
    #[test]
    fn decoding_skips_unknown_parts_and_fails_on_an_unknown_color() {
        let decode = |text: &str| serde_json::from_str::<SidebarTint>(text);
        assert_eq!(
            decode(r#"{"color":"red","on":["strip","name"]}"#).unwrap(),
            SidebarTint::new(TintColor::Red, TintParts::NAME)
        );
        for icon in [r#"{"color":"red"}"#, r#"{"color":"red","on":"icon"}"#] {
            assert_eq!(decode(icon).unwrap(), SidebarTint::on_icon(TintColor::Red), "{icon}");
        }
        assert!(decode(r#"{"color":"teal","on":["icon"]}"#).is_err());
    }

    /// `updateResolvesAgainstTheCurrentTint`.
    #[test]
    fn an_update_resolves_against_the_current_tint() {
        let named = SidebarTint::new(TintColor::Green, TintParts::NAME);
        assert_eq!(
            TintUpdate::Set(TintColor::Red, None).resolved(Some(named)),
            Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
        );
        assert_eq!(
            TintUpdate::Set(TintColor::Red, None).resolved(None),
            Some(SidebarTint::on_icon(TintColor::Red))
        );
        assert_eq!(
            TintUpdate::Set(TintColor::Red, Some(TintParts::BOTH)).resolved(Some(named)),
            Some(SidebarTint::new(TintColor::Red, TintParts::BOTH))
        );
        assert_eq!(TintUpdate::Clear.resolved(Some(named)), None);
    }
}
