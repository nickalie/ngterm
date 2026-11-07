import Foundation
import NgtermCore
import NgtermDesign

/// A status as a row of the chat shows it: its glyph, its color and the word that goes with them.
internal struct ChatStatusLook {
    internal let shape: StatusShape
    internal let color: ColorToken
    internal let word: String

    /// A tool's status: Running on the connecting ring, Done on the healthy circle, Failed on the failed triangle.
    internal static func tool(_ status: ChatToolStatus) -> Self {
        switch status {
        case .running:
            Self(shape: .ring, color: Palette.connecting, word: String(localized: "Running"))

        case .ok:
            Self(shape: .circle, color: Palette.healthy, word: String(localized: "Done"))

        case .error:
            Self(shape: .triangle, color: Palette.failed, word: String(localized: "Failed"))
        }
    }

    /// A question's status: Needs you while it waits, then Answered, or Failed.
    internal static func question(_ status: ChatToolStatus) -> Self {
        switch status {
        case .running:
            Self(shape: .diamond, color: Palette.attention, word: StatusTone.attention.word)

        case .ok:
            Self(shape: .circle, color: Palette.healthy, word: String(localized: "Answered"))

        case .error:
            Self(shape: .triangle, color: Palette.failed, word: String(localized: "Failed"))
        }
    }
}
