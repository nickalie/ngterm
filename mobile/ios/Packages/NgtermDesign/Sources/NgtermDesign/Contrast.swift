/// WCAG 2's contrast: the ratio, and the minimums the design system promises (DESIGN.md > Color).
public enum Contrast {
    /// Body text, and every status color, which is text next to its glyph.
    public static let text = 4.5
    /// Text with Increase Contrast on (WCAG's AAA).
    public static let increasedText = 7.0
    /// A glyph alone or a control's edge.
    public static let graphic = 3.0

    private static let flare = 0.05

    /// WCAG 2's contrast ratio of two opaque colors, from 1 to 21.
    public static func ratio(_ first: RGB, _ second: RGB) -> Double {
        let lighter = max(first.luminance, second.luminance) + flare
        let darker = min(first.luminance, second.luminance) + flare
        return lighter / darker
    }
}
