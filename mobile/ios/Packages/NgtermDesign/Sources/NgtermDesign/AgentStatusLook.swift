import Foundation

/// How an agent's status is drawn: its word, glyph and color.
public struct AgentStatusLook: Hashable, Sendable {
    /// The word: the state's, or the origin's own name for a state this build doesn't know.
    public let word: String
    /// The glyph.
    public let shape: StatusShape
    /// The color, readable on every ground it was resolved for.
    public let color: RGB

    /// The look of `status` (the origin's wire name) in `appearance`, with the `#rrggbb` color and the shape its origin
    /// sets (`colorHex`, `shape`).
    ///
    /// The origin's color is the user's choice and may not contrast, so it is lifted until it reads on every system
    /// ground in `appearance` (the word is always there besides). What the origin leaves unset comes from the status's
    /// ``AgentState``, then the circle and the idle color.
    @preconcurrency
    @MainActor
    public init(status: String?, colorHex: String?, shape: String?, in appearance: Appearance) {
        let state = AgentState.fromWire(status)
        word = state?.word ?? status ?? StatusTone.idle.word
        if let shape, !shape.trimmingCharacters(in: .whitespaces).isEmpty {
            self.shape = StatusShape.fromWire(shape)
        } else {
            self.shape = state?.shape ?? .circle
        }
        if let origin = RGB(parsing: colorHex) {
            color = origin.readable(on: Palette.grounds(in: appearance), minimum: appearance.textContrast)
        } else {
            color = (state?.tone ?? .idle).color.resolved(in: appearance)
        }
    }
}
