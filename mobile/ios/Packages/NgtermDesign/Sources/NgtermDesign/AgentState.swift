import Foundation

/// An agent's status as its origin names it (ngterm-core's `AgentStatus` wire names).
///
/// It carries the word, glyph and tone the app gives the status when the origin sets none. Desktop falls back to orange
/// for blocked and green for completed; the app's defaults are its own tones in those roles: working is the healthy
/// verdigris, waiting the attention diamond, done the verdigris star.
public enum AgentState: CaseIterable, Sendable {
    /// The agent is working.
    case active
    /// The agent waits for you.
    case blocked
    /// The agent finished.
    case completed
    /// Nothing is happening.
    case idle

    /// The state's name on the wire: its case's name.
    public var wireName: String {
        "\(self)"
    }

    /// The state's word.
    public var word: String {
        switch self {
        case .active:
            String(localized: "Working", bundle: .module)

        case .blocked:
            String(localized: "Waiting", bundle: .module)

        case .completed:
            String(localized: "Done", bundle: .module)

        case .idle:
            StatusTone.idle.word
        }
    }

    /// The glyph when the origin sets none.
    public var shape: StatusShape {
        switch self {
        case .active:
            .circle

        case .blocked:
            .diamond

        case .completed:
            .star

        case .idle:
            .capsule
        }
    }

    /// The tone whose color the state takes when the origin sets none.
    public var tone: StatusTone {
        switch self {
        case .active, .completed:
            .online

        case .blocked:
            .attention

        case .idle:
            .idle
        }
    }

    /// The state an origin names (its wire spelling, any case); `nil` for none, or for one this build doesn't know.
    public static func fromWire(_ name: String?) -> Self? {
        guard let name else {
            return nil
        }
        let spelled = name.trimmingCharacters(in: .whitespaces).lowercased()
        return allCases.first { $0.wireName == spelled }
    }
}
