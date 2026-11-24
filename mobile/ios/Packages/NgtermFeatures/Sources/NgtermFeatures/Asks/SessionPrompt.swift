/// What the sheet over a session shows: its first open ask, or once none is open, its first open pick.
internal enum SessionPrompt: Hashable, Identifiable {
    /// An ask (``AskQuestion``).
    case ask(OpenAsk)
    /// A pick (``PickQuestion``).
    case pick(OpenPick)

    /// Its name among every prompt.
    internal var id: String {
        switch self {
        case .ask(let open):
            "ask:\(open.id)"

        case .pick(let open):
            "pick:\(open.id)"
        }
    }
}
