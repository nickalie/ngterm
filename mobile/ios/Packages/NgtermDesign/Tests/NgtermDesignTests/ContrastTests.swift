import NgtermDesign
import Testing

/// DESIGN.md's color promises, checked against the system grounds as UIKit resolves them.
///
/// Every text color reaches 4.5:1 on every ground (7:1 with Increase Contrast), on the light and the dark side.
@MainActor
@Suite("contrast")
internal struct ContrastTests {
    private static let textTokens: [(name: String, token: ColorToken)] =
        [("tint", Palette.tint), ("healthy", Palette.healthy), ("secondaryText", Palette.secondaryText)]
        + StatusTone.ordered.map { tone in ("\(tone)", tone.color) }

    private func expectContrast(
        _ what: String,
        _ foreground: RGB,
        _ background: RGB,
        minimum: Double,
        sourceLocation: SourceLocation = #_sourceLocation
    ) {
        let ratio = Contrast.ratio(foreground, background)
        #expect(
            ratio >= minimum,
            "\(what): \(foreground) on \(background) is \(ratio), under \(minimum)",
            sourceLocation: sourceLocation
        )
    }

    @Test("every text color reads on every system ground", arguments: Appearance.allCases)
    internal func textColorsReadOnEveryGround(appearance: Appearance) {
        for (name, token) in Self.textTokens {
            for (ground, color) in Palette.grounds {
                expectContrast(
                    "\(appearance) \(name) on \(ground)",
                    token.resolved(in: appearance),
                    RGB(resolving: color, in: appearance),
                    minimum: appearance.textContrast
                )
            }
        }
    }

    @Test("every agent state's default reads on every system ground", arguments: Appearance.allCases)
    internal func agentStatesReadOnEveryGround(appearance: Appearance) {
        for state in AgentState.allCases {
            let look = AgentStatusLook(status: state.wireName, colorHex: nil, shape: nil, in: appearance)
            for (ground, groundColor) in zip(Palette.grounds.map(\.name), Palette.grounds(in: appearance)) {
                expectContrast(
                    "\(appearance) \(state) on \(ground)",
                    look.color,
                    groundColor,
                    minimum: appearance.textContrast
                )
            }
        }
    }

    @Test("a prominent button's label reads on the tint", arguments: Appearance.allCases)
    internal func onTintReadsOnTint(appearance: Appearance) {
        expectContrast(
            "\(appearance) onTint on tint",
            Palette.onTint.resolved(in: appearance),
            Palette.tint.resolved(in: appearance),
            minimum: appearance.textContrast
        )
    }

    @Test("every terminal palette's text reads and its cursor shows", arguments: TerminalPalette.builtIn)
    internal func terminalPalettesRead(palette: TerminalPalette) {
        expectContrast("\(palette.id) text", palette.foreground, palette.background, minimum: Contrast.text)
        expectContrast("\(palette.id) cursor", palette.cursor, palette.background, minimum: Contrast.graphic)
        expectContrast(
            "\(palette.id) text on the selection",
            palette.foreground,
            palette.selectionBackground,
            minimum: Contrast.text
        )
    }

    @Test("an origin's color is lifted until it reads on every ground at once", arguments: Appearance.allCases)
    internal func originColorsAreMadeReadable(appearance: Appearance) {
        let levels = (0...8).map { Double($0) / 8 }
        let grid = levels.flatMap { red in
            levels.flatMap { green in levels.map { blue in RGB(red: red, green: green, blue: blue) } }
        }
        let grounds = Palette.grounds(in: appearance)
        for color in grid + ["#f84050", "#f8f8f0"] {
            let look = AgentStatusLook(status: "active", colorHex: "\(color)", shape: nil, in: appearance)
            for ground in grounds {
                expectContrast("\(appearance) \(color) lifted", look.color, ground, minimum: appearance.textContrast)
            }
        }
    }

    @Test("a readable color is left alone, an unreadable one moves toward the far end")
    internal func readableLeavesOrLifts() {
        let dark: [RGB] = ["#000000"]
        let verdigris: RGB = "#5fbfaa"
        #expect(verdigris.readable(on: dark) == verdigris)
        let dim: RGB = "#222222"
        let lifted = dim.readable(on: dark)
        #expect(Contrast.ratio(lifted, dark[0]) >= Contrast.text)
        #expect(lifted.red > dim.red, "moved toward white on a dark ground")
        let paleGrey: RGB = "#f0f0f0"
        let pale = paleGrey.readable(on: [.white])
        #expect(Contrast.ratio(pale, .white) >= Contrast.text)
        #expect(pale.red < 0.94, "moved toward black on a light ground")
    }

    @Test("WCAG's ratio has its known ends")
    internal func contrastRatioEnds() {
        #expect(abs(Contrast.ratio(.white, .black) - 21) < 0.001)
        #expect(abs(Contrast.ratio(.white, .white) - 1) < 0.001)
    }
}
