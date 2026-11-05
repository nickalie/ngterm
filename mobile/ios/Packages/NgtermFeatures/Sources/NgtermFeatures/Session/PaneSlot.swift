import NgtermCore
import NgtermDesign
import NgtermTerminal
import SwiftUI

/// A pane's place in the session view (mobile/android's `PaneSlot`): its terminal once it is shown (the slot's first
/// size shows it), under its cover when it can't be used as it is.
///
/// The selected one (`isFocused`) draws its cursor as it is, the others' go hollow, and only it says its cursor's
/// line to VoiceOver. A touch on it, or its taking the focus, selects it; `swipe` takes a finger's sideways swipe (a
/// phone's next or previous pane).
internal struct PaneSlot: View {
    /// What a slot does, the same for every slot of a session.
    internal struct Actions {
        internal let model: SessionModel
        internal let keyboard: TerminalKeyboard
        internal let machine: String
        internal let look: TerminalLook
        internal let select: @MainActor (String) -> Void
        internal let swipe: (@MainActor (Bool) -> Void)?

        /// The same, a sideways drag only scrolling: one pane on screen of many is all a swipe moves between.
        internal var withoutSwipe: Self {
            Self(model: model, keyboard: keyboard, machine: machine, look: look, select: select, swipe: nil)
        }
    }

    @Environment(\.displayScale)
    private var displayScale

    internal let spot: PaneSpot
    internal let isFocused: Bool
    internal let actions: Actions

    internal var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            ZStack {
                Color(uiColor: actions.look.palette.background.uiColor)
                    .accessibilityHidden(true)
                if let shown = spot.shown {
                    terminal(shown)
                        .covered(by: veil(size: size))
                }
                if spot.shown == nil, let veil = veil(size: size) {
                    veil
                }
                if spot.cover == .reconnecting {
                    ReconnectingStrip(machine: actions.machine)
                }
            }
            .task(id: showKey(size: size)) {
                if spot.shown == nil, spot.failure == nil, let grid = grid(fitting: size) {
                    actions.model.show(spot, size: grid)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("pane-\(spot.id)")
    }

    // what the show of a pane at `size` waits on
    private func showKey(size: CGSize) -> PaneShowKey {
        let waits = spot.shown == nil && spot.failure == nil
        return PaneShowKey(ref: spot.ref, width: size.width, height: size.height, waits: waits)
    }

    private func terminal(_ shown: ShownPane) -> some View {
        let paneId = spot.id
        let select = actions.select
        let look = actions.look
        return TerminalViewRepresentable(
            pane: shown.client,
            feed: shown.feed,
            palette: look.palette,
            textSize: look.textSize,
            faces: look.faces,
            focused: isFocused
        )
        .keyboard(actions.keyboard)
        .onSelect { select(paneId) }
        .onSwipe(actions.swipe)
        .readsAloud(isFocused && spot.cover?.veils != true)
        .id(spot.ref)
    }

    private func veil(size: CGSize) -> Cover? {
        guard let cover = spot.cover else {
            return nil
        }
        let model = actions.model
        let covered = spot
        return PaneCovers.veil(
            cover,
            machine: actions.machine,
            onTakeLead: { model.takeLead(covered) },
            onDetach: { model.detach() },
            onRetry: {
                if let grid = grid(fitting: size) {
                    model.retry(covered, size: grid)
                }
            }
        )
    }

    // the grid the slot's size holds at the terminal's text size, as its view will lay it out
    private func grid(fitting size: CGSize) -> PaneSize? {
        TerminalFont(size: actions.look.textSize, faces: actions.look.faces).grid(fitting: size, scale: displayScale)
    }
}
