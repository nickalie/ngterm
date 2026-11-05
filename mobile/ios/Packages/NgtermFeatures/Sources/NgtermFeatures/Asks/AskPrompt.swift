import NgtermDesign
import SwiftUI

/// The first open ask of the session a view shows, in a sheet over it (mobile/android's `AskPrompt`; the ask rule: an
/// ask out of sight is a notification instead).
///
/// The view tells ``Asks`` whether its session is on screen: shown, and its window not in the background. The answer,
/// or a swipe that dismisses the sheet (which the origin gets as Escape), goes to the core once; a sheet whose ask
/// went (answered elsewhere, dismissed, taken back) goes by itself, and so does one whose view goes (another session
/// opened in its place, its window closed), leaving its ask open. An ask's draft outlives its sheet while the ask is
/// open, so one that left the screen comes back as the user left it.
internal struct AskPrompt: ViewModifier {
    @Environment(\.scenePhase)
    private var phase

    internal let graph: AppGraph
    internal let key: SessionKey
    @State private var shown: OpenAsk?
    @State private var drafts: [String: AskDraft] = [:]
    @State private var isVisible = false
    @State private var presence = AskPresence()

    private var first: OpenAsk? {
        graph.asks.open.first { $0.key == key }
    }

    // a sheet the user swiped away escapes its ask; one closed because its ask went is left as it is
    private var presented: Binding<OpenAsk?> {
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
                let ids = Set(graph.asks.open.map(\.id))
                drafts = drafts.filter { ids.contains($0.key) }
            }
            .sheet(item: presented) { open in
                AskQuestion(ask: open.ask, draft: draft(of: open)) { reply in
                    graph.asks.answer(open.key, askId: open.ask.id, reply: reply)
                }
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
    private func escapeIfSwiped(_ gone: OpenAsk) {
        let view = presence
        let asks = graph.asks
        Task { @MainActor in
            guard view.isShown, asks.find(gone.key, askId: gone.ask.id) != nil else {
                return
            }
            asks.answer(gone.key, askId: gone.ask.id, reply: AskDraft.escaped)
        }
    }

    private func draft(of open: OpenAsk) -> Binding<AskDraft> {
        Binding {
            drafts[open.id] ?? AskDraft(open.ask)
        } set: { draft in
            drafts[open.id] = draft
        }
    }
}

extension View {
    /// Shows the open asks of the session `key` of `graph` in a sheet over this view, while it shows.
    public func askPrompt(_ graph: AppGraph, session key: SessionKey) -> some View {
        modifier(AskPrompt(graph: graph, key: key))
    }
}
