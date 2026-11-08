import NgtermDesign
import Testing
import UIKit

/// The status system: a glyph, a color and a word for each status, and the origin's names read as Android reads them.
@MainActor
@Suite("status")
internal struct StatusTests {
    @Test("no two tones share a glyph, so none rests on color")
    internal func everyToneHasItsOwnGlyph() {
        let shapes = StatusTone.allCases.map(\.shape)
        #expect(Set(shapes).count == shapes.count)
        #expect(Set(StatusTone.ordered) == Set(StatusTone.allCases))
    }

    @Test("every glyph is an SF Symbol the system has", arguments: StatusShape.allCases)
    internal func everyGlyphExists(shape: StatusShape) {
        #expect(UIImage(systemName: shape.symbolName) != nil, "\(shape.symbolName)")
    }

    @Test("a shape follows the origin's wire names, and the app's own are not among them")
    internal func shapesFromTheWire() {
        #expect(StatusShape.fromWire("star") == .star)
        #expect(StatusShape.fromWire("Capsule") == .capsule)
        #expect(StatusShape.fromWire(nil) == .circle)
        #expect(StatusShape.fromWire("hexagon") == .circle)
        #expect(StatusShape.fromWire("ring") == .circle)
        #expect(StatusShape.fromWire("slash") == .circle)
        #expect(StatusShape.fromWire("triangle-down") == .triangleDown)
        #expect(StatusShape.fromWire("triangleDown") == .circle)
        #expect(StatusShape.fromWire("bolt") == .bolt)
        #expect(StatusShape.fromWire("Bell") == .bell)
        #expect(
            StatusShape.wire.map(\.wireName) == [
                "circle", "square", "triangle", "triangle-down", "play", "diamond", "capsule", "seal", "star", "bolt",
                "bell",
            ]
        )
        #expect(Set(StatusShape.wire.map(\.symbolName)).count == StatusShape.wire.count)
    }

    @Test("an agent state follows the origin's wire names")
    internal func statesFromTheWire() {
        #expect(AgentState.fromWire("blocked") == .blocked)
        #expect(AgentState.fromWire(" Completed ") == .completed)
        #expect(AgentState.fromWire("reviewing") == nil)
        #expect(AgentState.fromWire(nil) == nil)
    }

    @Test("an agent's look takes the origin's shape and color, and the state's for what it leaves unset")
    internal func agentLook() {
        let light = Appearance(isDark: false, isIncreasedContrast: false)
        let waiting = AgentStatusLook(status: "blocked", colorHex: nil, shape: nil, in: light)
        #expect(waiting.word == "Waiting")
        #expect(waiting.shape == .diamond)
        #expect(waiting.color == Palette.attention.light)

        let custom = AgentStatusLook(status: "blocked", colorHex: "#1a6a5d", shape: "square", in: light)
        #expect(custom.shape == .square)
        #expect(custom.color.description == "#1a6a5d")

        let unknown = AgentStatusLook(status: "reviewing", colorHex: "red", shape: " ", in: light)
        #expect(unknown.word == "reviewing")
        #expect(unknown.shape == .circle)
        #expect(unknown.color == Palette.idle.light)
    }

    @Test("a status hex is the origin's spelling and nothing else")
    internal func parsingHex() {
        let orange: RGB = "#ff8800"
        #expect(RGB(parsing: "#ff8800") == orange)
        #expect(RGB(parsing: "FF8800") == orange)
        #expect(RGB(parsing: nil) == nil)
        #expect(RGB(parsing: "red") == nil)
        #expect(RGB(parsing: "#fff") == nil)
        #expect(RGB(parsing: "#gg0000") == nil)
        #expect(orange.description == "#ff8800")
    }
}
