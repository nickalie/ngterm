import NgtermDesign
import SwiftUI

/// The first open ask of the session a view shows, in a sheet over it (mobile/android's `AskPrompt`; the ask rule: an
/// ask out of sight is a notification instead), and once none is open, its first open pick (mobile/android's
/// `PickPrompt`; a pick out of sight waits).
///
/// The view tells ``Asks`` whether its session is on screen: shown, and its window not in the background. The answer,
/// or a swipe that dismisses the sheet (which the origin gets as Escape, or a pick as a cancel), goes to the core
/// once; a sheet whose ask or pick went (answered elsewhere, dismissed, taken back) goes by itself, and so does one
/// whose view goes (another session opened in its place, its window closed), leaving its ask or pick open. An ask's
/// draft is ``Asks``', kept with the open ask rather than with this view, so one that left the screen comes back as
/// the user left it.
internal struct AskPrompt: ViewModifier {
    @Environment(\.scenePhase)
    private var phase

    internal let graph: AppGraph
    internal let key: SessionKey
    @State private var shown: SessionPrompt?
    @State private var isVisible = false
    @State private var presence = AskPresence()

    private var first: SessionPrompt? {
        if let ask = graph.asks.open.first(where: { $0.key == key }) {
            return .ask(ask)
        }
        return graph.picks.open.first { $0.key == key }.map(SessionPrompt.pick)
    }

    // a sheet the user swiped away escapes its ask or cancels its pick; one closed because its prompt went is left be
    private var presented: Binding<SessionPrompt?> {
        Binding {
            shown
        } set: { open in
            if open == nil, let gone = shown {
                escapeIfSwiped(gone)
            }
            shown = open
        }
    }

    internal func body(content: Content) -> some View {
        content
            .onAppear {
                isVisible = true
                report()
            }
            .onDisappear {
                isVisible = false
                report()
            }
            .onChange(of: phase) {
                report()
            }
            .onChange(of: first, initial: true) { _, open in
                shown = open
            }
            .sheet(item: presented) { prompt in
                question(prompt)
                    .ngtermTheme()
            }
    }

    private func report() {
        let isShown = isVisible && phase != .background
        presence.isShown = isShown
        graph.asks.showing(key, isShown: isShown)
    }

    // A sheet also goes as the view it hangs on goes, which SwiftUI may tell through the same binding, and before the
    // view's own disappearance: decided a moment later, only a sheet gone while its view still shows was swiped away.
    private func escapeIfSwiped(_ gone: SessionPrompt) {
        let view = presence
        let asks = graph.asks
        let picks = graph.picks
        Task { @MainActor in
            guard view.isShown else {
                return
            }
            switch gone {
            case .ask(let open):
                if asks.find(open.key, askId: open.ask.id) != nil {
                    asks.answer(open.key, askId: open.ask.id, reply: AskDraft.escaped)
                }

            case .pick(let open):
                if picks.find(open.key, pickId: open.pick.id) != nil {
                    picks.answer(open.key, pickId: open.pick.id, reply: .cancelled)
                }
            }
        }
    }

    @ViewBuilder
    private func question(_ prompt: SessionPrompt) -> some View {
        switch prompt {
        case .ask(let open):
            AskQuestion(ask: open.ask, draft: draft(of: open)) { reply in
                graph.asks.answer(open.key, askId: open.ask.id, reply: reply)
            }

        case .pick(let open):
            PickQuestion(pick: open.pick) { reply in
                graph.picks.answer(open.key, pickId: open.pick.id, reply: reply)
            }
        }
    }

    private func draft(of open: OpenAsk) -> Binding<AskDraft> {
        let asks = graph.asks
        return Binding {
            asks.draft(of: open)
        } set: { draft in
            asks.keep(draft, of: open)
        }
    }
}

extension View {
    /// Shows the open asks of the session `key` of `graph` in a sheet over this view, while it shows, then its open
    /// picks.
    public func askPrompt(_ graph: AppGraph, session key: SessionKey) -> some View {
        modifier(AskPrompt(graph: graph, key: key))
    }
}
