import Foundation

/// The glyphs a status is drawn with, as SF Symbols.
///
/// Six are upstream's status shapes as an origin names them (ngterm-core's `StatusShape`: ``wire``); ``ring`` and
/// ``slash`` are the app's own, for a connection on its way and one out of reach. The same eight as the Android app's.
public enum StatusShape: CaseIterable, Sendable {
    /// A filled capsule.
    case capsule
    /// A filled circle.
    case circle
    /// A filled diamond.
    case diamond
    /// An open ring: on its way.
    case ring
    /// A struck ring: out of reach.
    case slash
    /// A filled square.
    case square
    /// A filled star.
    case star
    /// A filled triangle.
    case triangle

    /// Upstream's six, in its order: the shapes an origin can name.
    public static let wire: [Self] = [.circle, .square, .triangle, .diamond, .capsule, .star]

    /// The shape's name on the wire: its case's name.
    public var wireName: String {
        "\(self)"
    }

    /// The SF Symbol that draws the shape.
    public var symbolName: String {
        switch self {
        case .capsule:
            "capsule.fill"

        case .circle:
            "circle.fill"

        case .diamond:
            "diamond.fill"

        case .ring:
            "circle"

        case .slash:
            "circle.slash"

        case .square:
            "square.fill"

        case .star:
            "star.fill"

        case .triangle:
            "triangle.fill"
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
