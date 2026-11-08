import Foundation

/// The glyphs a status is drawn with, as SF Symbols.
///
/// Eleven are upstream's status shapes as an origin names them (ngterm-core's `StatusShape`: ``wire``); ``ring`` and
/// ``slash`` are the app's own, for a connection on its way and one out of reach. The same thirteen as the Android
/// app's.
public enum StatusShape: CaseIterable, Sendable {
    /// A filled bell.
    case bell
    /// A filled lightning bolt.
    case bolt
    /// A filled capsule.
    case capsule
    /// A filled circle.
    case circle
    /// A filled diamond.
    case diamond
    /// A filled triangle pointing right.
    case play
    /// An open ring: on its way.
    case ring
    /// A filled circle with a scalloped rim.
    case seal
    /// A struck ring: out of reach.
    case slash
    /// A filled square.
    case square
    /// A filled star.
    case star
    /// A filled triangle.
    case triangle
    /// A filled triangle pointing down.
    case triangleDown

    /// Upstream's eleven, in its order: the shapes an origin can name.
    public static let wire: [Self] = [
        .circle, .square, .triangle, .triangleDown, .play, .diamond, .capsule, .seal, .star, .bolt, .bell,
    ]

    /// The shape's name on the wire: its case's name, but `triangle-down`.
    public var wireName: String {
        switch self {
        case .triangleDown:
            "triangle-down"

        default:
            "\(self)"
        }
    }

    /// The SF Symbol that draws the shape.
    public var symbolName: String {
        switch self {
        case .bell:
            "bell.fill"

        case .bolt:
            "bolt.fill"

        case .capsule:
            "capsule.fill"

        case .circle:
            "circle.fill"

        case .diamond:
            "diamond.fill"

        case .play:
            "play.fill"

        case .ring:
            "circle"

        case .seal:
            "seal.fill"

        case .slash:
            "circle.slash"

        case .square:
            "square.fill"

        case .star:
            "star.fill"

        case .triangle:
            "triangle.fill"

        case .triangleDown:
            "arrowtriangle.down.fill"
        }
    }

    /// The shape an origin names (its wire spelling, any case); the circle for none, or for one this build doesn't
    /// know.
    ///
    /// The app's own shapes are not wire names.
    public static func fromWire(_ name: String?) -> Self {
        guard let spelled = name?.trimmingCharacters(in: .whitespaces).lowercased() else {
            return .circle
        }
        return wire.first { $0.wireName == spelled } ?? .circle
    }
}
