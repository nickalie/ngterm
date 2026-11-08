//! Sidebar row tints (upstream's `SidebarTint.swift`, `ControlTintUpdate` in `ControlModes.swift` and
//! `AppStore+Tint.swift`): one color, a palette name or a `#rrggbb` hex, shown on a session or workspace row's icon,
//! its name, or both. A workspace's tint colors its own row only; the sessions inside keep theirs.
//!
//! The store seam here is what `session.tint`, `workspace.tint`, the row menus' Tint submenu and the palette's Tint
//! Session rows share: an unchanged value is a no-op, so a repeated set neither saves nor emits `tree.changed`.

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use super::status::is_valid_color_hex;
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
    Teal,
    Pink,
    Brown,
    Indigo,
    Mint,
}

impl TintColor {
    /// Every color, in upstream's declaration order (the menus', the palette's and the accepted-names list's).
    pub const ALL: [Self; 12] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Gray,
        Self::Teal,
        Self::Pink,
        Self::Brown,
        Self::Indigo,
        Self::Mint,
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
            Self::Teal => "teal",
            Self::Pink => "pink",
            Self::Brown => "brown",
            Self::Indigo => "indigo",
            Self::Mint => "mint",
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
            Self::Teal => "Teal",
            Self::Pink => "Pink",
            Self::Brown => "Brown",
            Self::Indigo => "Indigo",
            Self::Mint => "Mint",
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

/// A row tint's color (upstream's `TintValue`): a palette name, or a `#rrggbb` hex the control API takes. A hex is fixed
/// sRGB, so unlike a name it does not follow light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TintValue {
    Named(TintColor),
    /// The red, green and blue bytes.
    Hex([u8; 3]),
}

impl TintValue {
    /// Parses a name in any case, or a hex with or without `#` (upstream's `init(rawValue:)`); `None` for anything else.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(color) = TintColor::from_wire(&raw.to_lowercase()) {
            return Some(Self::Named(color));
        }
        if !is_valid_color_hex(raw) {
            return None;
        }
        let digits = raw.strip_prefix('#').unwrap_or(raw);
        let byte = |at: usize| {
            digits
                .get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        Some(Self::Hex([byte(0)?, byte(2)?, byte(4)?]))
    }

    /// The wire, snapshot and read-back spelling: the name, or the lowercase `#rrggbb`.
    #[must_use]
    pub fn wire(self) -> String {
        match self {
            Self::Named(color) => color.wire_name().to_owned(),
            Self::Hex([red, green, blue]) => format!("#{red:02x}{green:02x}{blue:02x}"),
        }
    }

    /// The accepted names plus the hex form, pipe-joined, for rejection text (upstream's `validValuesList`).
    #[must_use]
    pub fn valid_values_list() -> String {
        format!("{}|#rrggbb", TintColor::valid_names_list())
    }
}

impl From<TintColor> for TintValue {
    fn from(color: TintColor) -> Self {
        Self::Named(color)
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
    color: TintValue,
    parts: TintParts,
}

impl SidebarTint {
    /// A tint of `color` on `parts`, the icon when `parts` is empty: the one placement every tint must have.
    #[must_use]
    pub fn new(color: impl Into<TintValue>, parts: TintParts) -> Self {
        let parts = if parts.is_empty() { TintParts::ICON } else { parts };
        Self {
            color: color.into(),
            parts,
        }
    }

    /// A tint of `color` on the icon.
    #[must_use]
    pub fn on_icon(color: impl Into<TintValue>) -> Self {
        Self::new(color, TintParts::ICON)
    }

    #[must_use]
    pub fn color(self) -> TintValue {
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

    /// The tint a row with `current` gets for `color`: its own parts kept, an untinted row starting on `start` (the
    /// icon for the control API and the palette, the last Show On pick for the row menu).
    #[must_use]
    pub fn recolor(current: Option<Self>, color: TintValue, start: TintParts) -> Self {
        Self::new(color, current.map_or(start, Self::parts))
    }

    /// `current`'s color shown on `parts` instead. An untinted row stays untinted, so no row holds a placement it does
    /// not show.
    #[must_use]
    pub fn place(current: Option<Self>, parts: TintParts) -> Option<Self> {
        current.map(|tint| Self::new(tint.color, parts))
    }
}

/// Persisted as `{"color": "red", "on": ["icon"]}`, a hex as its lowercase `#rrggbb`. Decoding is lossy like the snapshots around it: an unknown color
/// fails, so the caller's lossy field drops the tint alone, while unknown parts are skipped and an empty, missing or
/// malformed list reads as the icon.
impl Serialize for SidebarTint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("SidebarTint", 2)?;
        state.serialize_field("color", &self.color.wire())?;
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
        let color = TintValue::parse(&stored.color)
            .ok_or_else(|| D::Error::custom(format!("unknown tint color: {}", stored.color)))?;
        let names: Vec<String> = serde_json::from_value(stored.on).unwrap_or_default();
        let parts = TintParts::of(names.iter().filter_map(|name| TintPart::from_wire(name)));
        Ok(Self::new(color, parts))
    }
}

/// A parsed `session.tint`/`workspace.tint` request (upstream's `ControlTintUpdate`): a color with optional parts, or
/// a clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintUpdate {
    Set(TintValue, Option<TintParts>),
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
            Self::Set(color, None) => Some(SidebarTint::recolor(current, color, TintParts::ICON)),
        }
    }
}

/// One choice of the row menu's Tint submenu (upstream's `TintMenuRequest.Choice`): a color (`None` is None), or where
/// the tinted rows show their own colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintChoice {
    Color(Option<TintValue>),
    Parts(TintParts),
}

impl TintChoice {
    /// The tint a row with `current` gets for this choice, an untinted row a color starts on `start`.
    #[must_use]
    pub fn applied(self, current: Option<SidebarTint>, start: TintParts) -> Option<SidebarTint> {
        match self {
            Self::Color(color) => color.map(|color| SidebarTint::recolor(current, color, start)),
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
    /// untinted one on `start`, a placement moves each tinted row's own color and leaves an untinted one alone. One
    /// save for the batch; returns whether any row changed.
    pub fn apply_session_tints(&mut self, choice: TintChoice, ids: &[Uuid], start: TintParts) -> bool {
        let mut changed = false;
        for session in self
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.sessions.iter_mut())
            .filter(|session| ids.contains(&session.id()))
        {
            let tint = choice.applied(session.tint, start);
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
        assert_eq!(TintPart::valid_names_list(), "icon,name");
    }

    /// `paletteListsTheTwelveNamesInMenuOrder`.
    #[test]
    fn the_palette_lists_the_twelve_names_in_menu_order() {
        assert_eq!(
            TintColor::valid_names_list(),
            "red|orange|yellow|green|blue|purple|gray|teal|pink|brown|indigo|mint"
        );
        assert_eq!(
            TintColor::ALL.map(TintColor::title).join(","),
            "Red,Orange,Yellow,Green,Blue,Purple,Gray,Teal,Pink,Brown,Indigo,Mint"
        );
        assert!(
            TintColor::ALL
                .iter()
                .all(|color| TintColor::from_wire(color.wire_name()) == Some(*color))
        );
    }

    const BLUE_HEX: TintValue = TintValue::Hex([0x3a, 0x7b, 0xd5]);

    /// `valueParsesANameOrANormalizedHex`, `valueRejectsAnythingElse`, `valueRawIsTheNameOrTheHex`.
    #[test]
    fn a_value_parses_a_name_or_a_normalized_hex_and_nothing_else() {
        for (raw, value) in [
            ("red", TintValue::Named(TintColor::Red)),
            ("Teal", TintValue::Named(TintColor::Teal)),
            ("#3A7BD5", BLUE_HEX),
            ("3a7bd5", BLUE_HEX),
        ] {
            assert_eq!(TintValue::parse(raw), Some(value), "{raw}");
        }
        for raw in ["", "maroon", "#3a7bd", "#3a7bd5ff", "#3a7bdz", "##3a7bd5"] {
            assert_eq!(TintValue::parse(raw), None, "{raw}");
        }
        assert_eq!(TintValue::Named(TintColor::Indigo).wire(), "indigo");
        assert_eq!(BLUE_HEX.wire(), "#3a7bd5");
        assert_eq!(
            TintValue::valid_values_list(),
            "red|orange|yellow|green|blue|purple|gray|teal|pink|brown|indigo|mint|#rrggbb"
        );
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
        let red = TintValue::Named(TintColor::Red);
        assert_eq!(
            SidebarTint::recolor(Some(named), red, TintParts::ICON),
            SidebarTint::new(TintColor::Red, TintParts::NAME)
        );
        assert_eq!(
            SidebarTint::recolor(None, red, TintParts::ICON),
            SidebarTint::on_icon(TintColor::Red)
        );
        assert_eq!(TintChoice::Color(None).applied(Some(named), TintParts::ICON), None);
        assert_eq!(
            SidebarTint::place(Some(SidebarTint::on_icon(TintColor::Blue)), TintParts::NAME),
            Some(SidebarTint::new(TintColor::Blue, TintParts::NAME))
        );
        assert_eq!(SidebarTint::place(None, TintParts::NAME), None);
        assert_eq!(TintChoice::Parts(TintParts::BOTH).applied(None, TintParts::BOTH), None);
    }

    /// `recolorTakesAHexAndKeepsParts`.
    #[test]
    fn recolor_takes_a_hex_and_keeps_the_parts() {
        let named = SidebarTint::new(TintColor::Green, TintParts::NAME);
        assert_eq!(
            SidebarTint::recolor(Some(named), BLUE_HEX, TintParts::ICON),
            SidebarTint::new(BLUE_HEX, TintParts::NAME)
        );
        assert_eq!(
            SidebarTint::recolor(
                Some(SidebarTint::new(BLUE_HEX, TintParts::NAME)),
                TintColor::Red.into(),
                TintParts::ICON
            ),
            SidebarTint::new(TintColor::Red, TintParts::NAME)
        );
    }

    /// `recolorStartsAnUntintedRowOnTheGivenPlacement`: the menu's last Show On pick starts an untinted row; a tinted
    /// one keeps its own.
    #[test]
    fn recolor_starts_an_untinted_row_on_the_given_placement() {
        let named = SidebarTint::new(TintColor::Green, TintParts::NAME);
        let red = TintValue::Named(TintColor::Red);
        assert_eq!(
            SidebarTint::recolor(None, red, TintParts::BOTH),
            SidebarTint::new(TintColor::Red, TintParts::BOTH)
        );
        assert_eq!(
            SidebarTint::recolor(Some(named), red, TintParts::ICON),
            SidebarTint::new(TintColor::Red, TintParts::NAME)
        );
        assert_eq!(
            TintChoice::Color(Some(red)).applied(None, TintParts::NAME),
            Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
        );
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
        for color in ["maroon", "#12345", "#1234567", "#12345g"] {
            let error = decode(&format!(r#"{{"color":"{color}","on":["icon"]}}"#)).unwrap_err();
            assert!(
                error.to_string().contains(&format!("unknown tint color: {color}")),
                "{error}"
            );
        }
    }

    /// `hexTintEncodesAsOneColorString`, `namedTintKeepsItsSnapshotSpelling`.
    #[test]
    fn a_hex_tint_is_one_lowercase_color_string_and_a_name_keeps_its_spelling() {
        let tint = SidebarTint::new(BLUE_HEX, TintParts::NAME);
        let text = serde_json::to_string(&tint).unwrap();
        assert_eq!(text, r##"{"color":"#3a7bd5","on":["name"]}"##);
        assert_eq!(serde_json::from_str::<SidebarTint>(&text).unwrap(), tint);
        assert_eq!(
            serde_json::from_str::<SidebarTint>(r##"{"color":"#3A7BD5","on":["name"]}"##).unwrap(),
            tint
        );
        let mint = serde_json::to_string(&SidebarTint::on_icon(TintColor::Mint)).unwrap();
        assert_eq!(mint, r#"{"color":"mint","on":["icon"]}"#);
        assert_eq!(
            SidebarTint::on_icon(TintColor::Mint),
            SidebarTint::on_icon(TintValue::Named(TintColor::Mint))
        );
    }

    /// `updateResolvesAgainstTheCurrentTint`.
    #[test]
    fn an_update_resolves_against_the_current_tint() {
        let named = SidebarTint::new(TintColor::Green, TintParts::NAME);
        assert_eq!(
            TintUpdate::Set(TintColor::Red.into(), None).resolved(Some(named)),
            Some(SidebarTint::new(TintColor::Red, TintParts::NAME))
        );
        assert_eq!(
            TintUpdate::Set(TintColor::Red.into(), None).resolved(None),
            Some(SidebarTint::on_icon(TintColor::Red))
        );
        assert_eq!(
            TintUpdate::Set(TintColor::Red.into(), Some(TintParts::BOTH)).resolved(Some(named)),
            Some(SidebarTint::new(TintColor::Red, TintParts::BOTH))
        );
        assert_eq!(TintUpdate::Clear.resolved(Some(named)), None);
        assert_eq!(
            TintUpdate::Set(BLUE_HEX, None).resolved(Some(named)),
            Some(SidebarTint::new(BLUE_HEX, TintParts::NAME))
        );
    }
}
