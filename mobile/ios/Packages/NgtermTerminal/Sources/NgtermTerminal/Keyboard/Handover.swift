/// The focus one terminal hands the next as the session view moves the selection (``TerminalKeyboard/handOver()``).
internal enum Handover {
    /// The focus, the soft keyboard closed.
    case focus
    /// The focus with the soft keyboard open.
    case withKeyboard
}
