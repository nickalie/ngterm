import NgtermCore
import NgtermTerminal

/// A pane this app shows: the core's handle on it, and the feed that carries its frame events to the terminal view
/// showing it now (mobile/android's `TerminalState`, whose drawing state lives in the view on iOS).
@preconcurrency
@MainActor
public final class ShownPane {
    /// The core's handle: input goes to its terminal, frames come from it.
    public let client: any PaneClient
    /// Where its `PaneDirty` and `PaneAttached` go.
    public let feed = TerminalFeed()

    /// The pane behind `client`, with a feed of its own.
    public init(client: any PaneClient) {
        self.client = client
    }

    deinit {
        // the feed holds its view weakly
    }
}
