import Foundation

/// An opaque sRGB color, its channels from 0 to 1.
///
/// The form the design system's colors are defined, compared and checked in (``Contrast/ratio(_:_:)``), before SwiftUI
/// or UIKit draw them. A literal is its `#rrggbb` spelling, as DESIGN.md writes it.
public struct RGB: Hashable, Sendable, CustomStringConvertible, ExpressibleByStringLiteral {
    /// White, the light end of the lift in ``readable(on:minimum:)``.
    public static let white: Self = "#ffffff"
    /// Black, the dark end of the lift in ``readable(on:minimum:)``.
    public static let black: Self = "#000000"

    private static let channelMax = 255.0
    private static let hexDigits = 6
    private static let hexRadix = 16
    private static let byteDigits = 2
    private static let redOffset = 0
    private static let greenOffset = 2
    private static let blueOffset = 4
    private static let liftSteps = 20
    // WCAG 2's sRGB transfer function and luminance weights
    private static let linearKnee = 0.04045
    private static let linearSlope = 12.92
    private static let gammaOffset = 0.055
    private static let gammaScale = 1.055
    private static let gamma = 2.4
    private static let redWeight = 0.2126
    private static let greenWeight = 0.7152
    private static let blueWeight = 0.0722

    /// The red channel, 0 to 1.
    public let red: Double
    /// The green channel, 0 to 1.
    public let green: Double
    /// The blue channel, 0 to 1.
    public let blue: Double

    /// The `#rrggbb` spelling, lower case.
    public var description: String {
        String(format: "#%02x%02x%02x", Self.byte(red), Self.byte(green), Self.byte(blue))
    }

    /// WCAG 2's relative luminance, 0 (black) to 1 (white).
    public var luminance: Double {
        func linear(_ channel: Double) -> Double {
            channel <= Self.linearKnee
                ? channel / Self.linearSlope
                : pow((channel + Self.gammaOffset) / Self.gammaScale, Self.gamma)
        }
        return Self.redWeight * linear(red) + Self.greenWeight * linear(green) + Self.blueWeight * linear(blue)
    }

    /// The color with these channels, each clamped to 0...1.
    public init(red: Double, green: Double, blue: Double) {
        self.red = min(max(red, 0), 1)
        self.green = min(max(green, 0), 1)
        self.blue = min(max(blue, 0), 1)
    }

    /// The color a status hex names as an origin sends it (`#rrggbb` or `rrggbb`); `nil` for anything else.
    public init?(parsing hex: String?) {
        guard let hex else {
            return nil
        }
        let digits = Array(hex.hasPrefix("#") ? hex.dropFirst() : Substring(hex))
        guard digits.count == Self.hexDigits, digits.allSatisfy(\.isHexDigit) else {
            return nil
        }
        func channel(at offset: Int) -> Double? {
            Int(String(digits[offset..<(offset + Self.byteDigits)]), radix: Self.hexRadix).map { value in
                Double(value) / Self.channelMax
            }
        }
        guard
            let redValue = channel(at: Self.redOffset),
            let greenValue = channel(at: Self.greenOffset),
            let blueValue = channel(at: Self.blueOffset)
        else {
            return nil
        }
        self.init(red: redValue, green: greenValue, blue: blueValue)
    }

    /// The color a `#rrggbb` literal names; anything else is a programming error.
    public init(stringLiteral value: String) {
        guard let color = Self(parsing: value) else {
            preconditionFailure("\(value) is not a #rrggbb color")
        }
        self = color
    }

    private static func byte(_ channel: Double) -> Int {
        Int((channel * channelMax).rounded())
    }

    /// The color a fraction `amount` (0 to 1) of the way from this one to `other`.
    public func mixed(with other: Self, by amount: Double) -> Self {
        Self(
            red: red + (other.red - red) * amount,
            green: green + (other.green - green) * amount,
            blue: blue + (other.blue - blue) * amount
        )
    }

    /// This color moved toward white or black, whichever contrasts the most with the worst of `grounds`, by the least
    /// that makes it reach `minimum` against every one of them; the color itself when it already does.
    ///
    /// For colors the app doesn't choose: an agent's status color, which its origin sends.
    public func readable(on grounds: [Self], minimum: Double = Contrast.text) -> Self {
        func reads(_ candidate: Self) -> Bool {
            grounds.allSatisfy { Contrast.ratio(candidate, $0) >= minimum }
        }
        if reads(self) {
            return self
        }
        let worst = { (end: Self) in grounds.map { Contrast.ratio(end, $0) }.min() ?? 0 }
        let target = worst(.white) >= worst(.black) ? Self.white : Self.black
        let candidates = (1...Self.liftSteps).lazy.map { step in
            mixed(with: target, by: Double(step) / Double(Self.liftSteps))
        }
        return candidates.first(where: reads) ?? target
    }
}
