/// Whether there is more of the conversation before what the chat shows.
public enum OlderPages: Equatable, Sendable {
    /// The conversation's start is shown.
    case atStart
    /// The last page could not be read, for `reason` (the core's words); Retry asks again.
    case failed(reason: String)
    /// A page is on its way.
    case loading
    /// Maybe: the list's top asks for a page.
    case more
}
