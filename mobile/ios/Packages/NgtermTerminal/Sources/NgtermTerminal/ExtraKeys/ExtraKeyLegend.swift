/// How an extra key's legend is drawn.
public enum ExtraKeyLegend: Equatable, Sendable {
    /// An SF Symbol (the arrows, the keyboard), by name.
    case image(String)
    /// A character compared as itself (`|`, `/`), in the terminal's face, as it will land.
    case symbol(String)
    /// A word, in the chrome's type, at one size across the bar.
    case word(String)
}
