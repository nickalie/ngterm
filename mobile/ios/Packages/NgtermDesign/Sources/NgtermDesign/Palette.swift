import UIKit

/// The app's own colors (DESIGN.md > Color).
///
/// The grounds, text and fills are the system's, so the chrome is iOS's; the app adds copper as the tint, verdigris for
/// healthy, and the status colors. `ContrastTests` checks every promise.
public enum Palette {
    /// The app's tint, copper, which is text on every ground too: the selection, buttons, links, controls' accents.
    public static let tint = ColorToken(
        light: "#984a19",
        dark: "#e3884f",
        lightIncreased: "#7c3c14",
        darkIncreased: "#efbb9a"
    )

    /// Text and glyphs on a fill of ``tint`` (a prominent button).
    public static let onTint = ColorToken(
        light: "#ffffff",
        dark: "#2a1406",
        lightIncreased: "#ffffff",
        darkIncreased: "#2a1406"
    )

    /// Verdigris, copper's patina: healthy, connected and working, and attached.
    public static let healthy = ColorToken(
        light: "#1a6a5d",
        dark: "#5fbfaa",
        lightIncreased: "#15574d",
        darkIncreased: "#90d3c4"
    )

    /// Brass: on its way (connecting, reconnecting).
    public static let connecting = ColorToken(
        light: "#7a5a0a",
        dark: "#d9b25a",
        lightIncreased: "#634908",
        darkIncreased: "#e1c27c"
    )

    /// Slate: not connected, and not trying.
    public static let offline = ColorToken(
        light: "#535f65",
        dark: "#9aa6ac",
        lightIncreased: "#454f54",
        darkIncreased: "#bfc7cb"
    )

    /// A brighter copper: waiting on you (an ask, a host key to confirm, a sign-in).
    public static let attention = ColorToken(
        light: "#984a19",
        dark: "#f0904f",
        lightIncreased: "#7c3c14",
        darkIncreased: "#f5b98f"
    )

    /// Oxide: failed (a refused key, an unreachable host).
    public static let failed = ColorToken(
        light: "#a83226",
        dark: "#f2867a",
        lightIncreased: "#8f2a20",
        darkIncreased: "#f7b6af"
    )

    /// Graphite grey: nothing happening (an agent with no status).
    public static let idle = ColorToken(
        light: "#47535a",
        dark: "#a9b4b9",
        lightIncreased: "#444f56",
        darkIncreased: "#bec7ca"
    )

    /// Secondary text: a row's address and its line, a note, a plate's label, a form's hint.
    ///
    /// The system's secondary label stays under 4.5:1 on the grouped grounds, so the app's secondary text is this
    /// grey, which reads on every ground as every text token does, and on an iPad sidebar's grey rows too.
    public static let secondaryText = ColorToken(
        light: "#58585e",
        dark: "#a0a0a6",
        lightIncreased: "#4a4a50",
        darkIncreased: "#c8c8ce"
    )

    /// The mark's copper (`resources/icons/ngterm.svg`): the app icon's fill only, never text.
    public static let brandCopper: RGB = "#cf6e31"

    /// The system grounds text and badges are put on: the plain and the grouped backgrounds, at every level.
    ///
    /// A screen, a list's rows, a sheet, a popover: every text promise of DESIGN.md holds on each of them.
    public static let grounds: [(name: String, color: UIColor)] = [
        ("systemBackground", .systemBackground),
        ("secondarySystemBackground", .secondarySystemBackground),
        ("tertiarySystemBackground", .tertiarySystemBackground),
        ("systemGroupedBackground", .systemGroupedBackground),
        ("secondarySystemGroupedBackground", .secondarySystemGroupedBackground),
        ("tertiarySystemGroupedBackground", .tertiarySystemGroupedBackground),
    ]

    /// ``grounds`` resolved in `appearance`.
    @preconcurrency
    @MainActor
    public static func grounds(in appearance: Appearance) -> [RGB] {
        grounds.map { RGB(resolving: $0.color, in: appearance) }
    }
}
