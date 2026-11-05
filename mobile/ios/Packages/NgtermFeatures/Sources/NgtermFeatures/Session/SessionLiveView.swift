import NgtermDesign
import NgtermTerminal
import SwiftUI

/// An attached session, live (mobile/android's `SessionScreen`): its toolbar (name, state and context, the pane
/// switcher, More: Detach, Open in New Window, End session), the origin's HUD, and its panes.
///
/// A compact width shows the selected pane alone and a swipe across it moves to the next or previous one; a regular
/// width shows the origin's split. One keyboard serves every pane, and its chords move between the panes and the
/// sessions. The selected pane is the only one anything typed reaches: a tab, a swipe, a chord and a touch on a pane
/// select it, and the keyboard's focus follows the selection (``TerminalKeyboard/handOver()``).
internal struct SessionLiveView: View {
    // the least height the HUD shows in: below it, it would take the terminal's last rows (a phone on its side with
    // the soft keyboard up)
    private static let hudRoom: CGFloat = 320

    internal let model: SessionModel
    internal let session: AttachedSession
    internal let focused: String?
    internal let isCompact: Bool
    internal let look: TerminalLook
    internal let actions: SessionActions
    @State private var isConfirmingEnd = false
    @State private var isForward = true
    // the layout the panes show in: it follows `isCompact` a moment later, once the keyboard's focus is handed over
    @State private var shownCompact: Bool

    internal var body: some View {
        GeometryReader { geometry in
            VStack(spacing: 0) {
                if let hud = session.live.hud, geometry.size.height >= Self.hudRoom {
                    HudBanner(hud: hud)
                        .padding(.horizontal, Spacing.small)
                        .padding(.vertical, Spacing.xSmall)
                }
                SessionPanesView(
                    session: session,
                    focused: focused,
                    isCompact: shownCompact,
                    actions: slotActions,
                    isForward: isForward
                )
            }
        }
        .background(Color(uiColor: look.palette.background.uiColor))
        .navigationTitle(session.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            toolbar
        }
        .safeAreaInset(edge: .bottom) {
            notice
        }
        .confirmingDestructive(
            String(localized: "End \(session.name)?"),
            isPresented: $isConfirmingEnd,
            message: String(
                localized: "It stops on \(session.machine), with everything running in it. This can't be undone."
            ),
            action: RowAction(String(localized: "End session"), perform: end)
        )
        .onAppear {
            model.keyboard.shortcuts = TerminalShortcuts(
                nextPane: { move(forward: true) },
                previousPane: { move(forward: false) },
                nextSession: { moveSession(forward: true) },
                previousSession: { moveSession(forward: false) }
            )
        }
        .onDisappear {
            // the moves hold this view, and through it the model that holds the keyboard
            model.keyboard.shortcuts = TerminalShortcuts()
        }
        .onChange(of: isCompact) { _, compact in
            // the other layout builds the selected pane's terminal anew: the old one, still the keyboard's, hands its
            // focus (and the soft keyboard) over to it before it goes
            model.keyboard.handOver()
            shownCompact = compact
        }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            SessionTitle(session: session)
        }
        let panes = session.live.layout?.panes ?? []
        if panes.count > 1 {
            ToolbarItem(placement: .topBarTrailing) {
                PaneSwitcher(
                    panes: panes,
                    selected: focused,
                    titles: session.live.panes.compactMapValues(\.title),
                    onSelect: select
                )
            }
        }
        ToolbarItem(placement: .topBarTrailing) {
            SessionMenu(
                name: session.name,
                onDetach: model.detach,
                onNewWindow: actions.openInNewWindow
            ) {
                isConfirmingEnd = true
            }
        }
    }

    @ViewBuilder private var notice: some View {
        if let words = model.notice {
            Banner(.failed, message: words) {
                model.dismissNotice()
            }
            .padding(.horizontal, Spacing.large)
            .padding(.vertical, Spacing.small)
        }
    }

    private var slotActions: PaneSlot.Actions {
        PaneSlot.Actions(
            model: model,
            keyboard: model.keyboard,
            machine: session.machine,
            look: look,
            select: select,
            swipe: move(forward:)
        )
    }

    /// The live `session` of `model`, its pane `focused` selected, in a compact width's layout when `isCompact`, its
    /// terminals in `look`, asking its window for `actions`.
    internal init(
        model: SessionModel,
        session: AttachedSession,
        focused: String?,
        isCompact: Bool,
        look: TerminalLook,
        actions: SessionActions
    ) {
        self.model = model
        self.session = session
        self.focused = focused
        self.isCompact = isCompact
        self.look = look
        self.actions = actions
        _shownCompact = State(initialValue: isCompact)
    }

    // selects `paneId`, the keyboard's focus following; a compact width slides it in from the side it lies on
    private func select(_ paneId: String) {
        let panes = session.live.layout?.panes ?? []
        let from = model.focusedPane.flatMap(panes.firstIndex(of:)) ?? 0
        isForward = (panes.firstIndex(of: paneId) ?? 0) >= from
        if model.select(paneId) {
            model.keyboard.handOver()
        }
    }

    private func move(forward: Bool) {
        guard let next = model.neighbor(forward: forward) else {
            return
        }
        select(next)
        isForward = forward
    }

    // ends the session, and closes the view once it ended: the listing may have dropped it first, leaving the view on
    // the session gone rather than let go (a close the view asked for already changes nothing)
    private func end() {
        Task {
            if await model.end() {
                actions.close()
            }
        }
    }

    private func moveSession(forward: Bool) {
        if let next = model.neighborSession(forward: forward) {
            actions.open(next)
        }
    }
}
