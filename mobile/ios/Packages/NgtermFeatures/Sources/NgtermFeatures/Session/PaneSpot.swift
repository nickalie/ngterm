import NgtermCore

/// One pane of a session as the session view places it (mobile/android's `PaneSpot`): which pane, its terminal once
/// it is shown, what its events say, and why it could not be shown; whether it shows as a chat, with its chat once
/// that is open.
@preconcurrency
@MainActor
public struct PaneSpot {
    /// Which pane.
    public let ref: PaneRef
    /// Its terminal, once it is shown.
    public let shown: ShownPane?
    /// What its events say.
    public let live: PaneLive?
    /// Why it could not be shown (the core's words).
    public let failure: String?
    /// Whether it shows as a chat.
    public let isChatView: Bool
    /// Its chat, once open.
    public let chat: ChatModel?

    /// The pane's id in its session.
    public var id: String {
        ref.paneId
    }

    /// What covers it, if anything: a chat only a failed show's, since reading a chat never takes control.
    public var cover: PaneCover? {
        let found = PaneCover.of(live, hasTerminal: shown != nil, failure: failure)
        guard isChatView else {
            return found
        }
        if case .failed = found {
            return found
        }
        return nil
    }

    /// The pane `ref`, `shown` or not yet, with its `live` state and its show's `failure`; in chat view
    /// (`isChatView`) with its `chat` once open.
    public init(
        ref: PaneRef,
        shown: ShownPane?,
        live: PaneLive?,
        failure: String?,
        isChatView: Bool = false,
        chat: ChatModel? = nil
    ) {
        self.ref = ref
        self.shown = shown
        self.live = live
        self.failure = failure
        self.isChatView = isChatView
        self.chat = chat
    }
}
