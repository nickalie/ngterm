/// How a line that opens names its tap for VoiceOver: what opening it shows, and what closing it hides.
internal struct ChatToggle {
    internal let isExpanded: Bool
    internal let show: String
    internal let hide: String
    internal let onToggle: @MainActor () -> Void
}
