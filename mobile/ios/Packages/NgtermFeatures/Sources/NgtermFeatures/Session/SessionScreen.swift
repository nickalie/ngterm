import NgtermDesign
import NgtermTerminal
import SwiftUI

/// The session view (mobile/android's `SessionsDestination` for an open session): the session live, or gone once it
/// ended on its machine while open, with Close.
///
/// A compact width shows one pane at a time, a regular width the origin's split. Once the session is let go some other
/// way (Detach, End session, a disconnect), the view asks its window to close it. While it shows, the screen stays
/// awake when Settings asks for it, its extra keys lay out as Settings says, and the session's open asks show in a
/// sheet over it (``AskPrompt``). Its caller gives it the session's identity (`.id`), so another session gets a view
/// of its own.
public struct SessionScreen: View {
    @Environment(\.colorScheme)
    private var colorScheme
    @Environment(\.horizontalSizeClass)
    private var sizeClass

    private let graph: AppGraph
    private let key: SessionKey
    private let actions: SessionActions
    @State private var model: SessionModel
    @State private var holder = UUID()

    /// The body of the view.
    public var body: some View {
        content
            .onAppear {
                graph.screenAwake.hold(holder, wanted: graph.settings.settings.keepScreenOn)
            }
            .onDisappear {
                graph.screenAwake.hold(holder, wanted: false)
            }
            .onChange(of: graph.settings.settings.keepScreenOn) { _, keepsOn in
                graph.screenAwake.hold(holder, wanted: keepsOn)
            }
            .onChange(of: graph.settings.settings.extraKeys, initial: true) { _, choice in
                model.keyboard.layout = ExtraKeysLayout(choice)
            }
            .askPrompt(graph, session: key)
    }

    @ViewBuilder private var content: some View {
        switch model.state {
        case .live(let session, let focused):
            SessionLiveView(
                model: model,
                session: session,
                focused: focused,
                isCompact: sizeClass != .regular,
                look: TerminalLook(graph.settings.settings, isDark: colorScheme == .dark),
                actions: actions
            )

        case .gone(let name, let machine):
            StatePanel(
                title: String(localized: "\(name) is gone"),
                systemImage: "terminal",
                message: String(localized: "It ended on \(machine), so there is nothing left to show."),
                action: RowAction(String(localized: "Close"), perform: actions.close)
            )

        case nil:
            Color.clear
                .onAppear {
                    actions.close()
                }
        }
    }

    /// The session `session` over `graph`, asking its window for `actions`; its panes type through `keyboard` (one of
    /// its own by default).
    public init(graph: AppGraph, session: SessionKey, actions: SessionActions, keyboard: TerminalKeyboard? = nil) {
        self.graph = graph
        key = session
        self.actions = actions
        _model = State(initialValue: SessionModel(graph: graph, key: session, keyboard: keyboard))
    }
}
