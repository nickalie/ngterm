import Foundation

/// The app's own statuses: a machine's connection, and what is asked of you.
///
/// Each has a color, a glyph and a word, which always go out together, so no status rests on color alone.
public enum StatusTone: CaseIterable, Sendable {
    /// Waiting on you: an ask, a host key to confirm, a sign-in.
    case attention
    /// Connecting or reconnecting.
    case connecting
    /// Failed: a refused key, an unreachable host.
    case failed
    /// Nothing happening: an agent with no status.
    case idle
    /// Not connected, and not trying.
    case offline
    /// Connected and well.
    case online

    /// The tones in DESIGN.md's order: the connection's, then what is asked of you, then idle.
    public static let ordered: [Self] = [.online, .connecting, .offline, .attention, .failed, .idle]

    /// The tone's glyph; no two tones share one.
    public var shape: StatusShape {
        switch self {
        case .attention:
            .diamond

        case .connecting:
            .ring

        case .failed:
            .triangle

        case .idle:
            .capsule

        case .offline:
            .slash

        case .online:
            .circle
        }
    }

    /// The tone's color.
    public var color: ColorToken {
        switch self {
        case .attention:
            Palette.attention

        case .connecting:
            Palette.connecting

        case .failed:
            Palette.failed

        case .idle:
            Palette.idle

        case .offline:
            Palette.offline

        case .online:
            Palette.healthy
        }
    }

    /// The tone's word.
    public var word: String {
        switch self {
        case .attention:
            String(localized: "Needs you", bundle: .module)

        case .connecting:
            String(localized: "Connecting", bundle: .module)

        case .failed:
            String(localized: "Failed", bundle: .module)

        case .idle:
            String(localized: "Idle", bundle: .module)

        case .offline:
            String(localized: "Offline", bundle: .module)

        case .online:
            String(localized: "Connected", bundle: .module)
        }
    }
}
