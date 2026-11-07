import NgtermCore
import NgtermDesign
import NgtermTerminal
import SwiftUI

/// A pane's place in the session view (mobile/android's `PaneSlot`): its terminal once it is shown (the slot's first
/// size shows it), under its cover when it can't be used as it is; or, in chat view, its chat in the terminal's place,
/// the terminal still laid out under it at the slot's grid, unfocused and out of sight, so a chat action that takes
/// control lays the program out for this device, under no cover but a failed show's.
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
                    if spot.isChatView {
                        terminal(shown, isChat: true)
                            .opacity(0)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                        chatContent
                            .covered(by: veil(size: size))
                    } else {
                        terminal(shown, isChat: false)
                            .covered(by: veil(size: size))
                    }
                }
                if spot.shown == nil, let veil = veil(size: size) {
                    veil
                } else if spot.shown == nil, spot.isChatView {
                    ChatLoading(machine: actions.machine)
                        .background(ChatGrounds.chat)
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

    // a pane in chat view: its chat once open, the backlog's wait until then; a touch on it selects the pane
    @ViewBuilder private var chatContent: some View {
        let paneId = spot.id
        let select = actions.select
        Group {
            if let chat = spot.chat {
                ChatScreen(
                    chat: chat,
                    machine: actions.machine,
                    keyboard: actions.keyboard,
                    isSelected: isFocused
                ) {
                    actions.model.pick(paneId, .terminal)
                }
                .id(spot.ref)
            } else {
                ChatLoading(machine: actions.machine)
                    .background(ChatGrounds.chat)
            }
        }
        .simultaneousGesture(
            TapGesture().onEnded {
                select(paneId)
            }
        )
    }

    // what the show of a pane at `size` waits on
    private func showKey(size: CGSize) -> PaneShowKey {
        let waits = spot.shown == nil && spot.failure == nil
        return PaneShowKey(ref: spot.ref, width: size.width, height: size.height, waits: waits)
    }

    // the pane's terminal; under a chat (`isChat`) it is never drawn focused, so it takes no keyboard
    private func terminal(_ shown: ShownPane, isChat: Bool) -> some View {
        let paneId = spot.id
        let select = actions.select
        let look = actions.look
        return TerminalViewRepresentable(
            pane: shown.client,
            feed: shown.feed,
            palette: look.palette,
            textSize: look.textSize,
            faces: look.faces,
            focused: isFocused && !isChat
        )
        .keyboard(actions.keyboard)
        .onSelect { select(paneId) }
        .onSwipe(isChat ? nil : actions.swipe)
        .readsAloud(!isChat && isFocused && spot.cover?.veils != true)
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
