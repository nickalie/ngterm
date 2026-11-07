import NgtermDesign
import SwiftUI

/// A live session's panes, or what stands in for them before its layout arrives (mobile/android's `Panes`).
///
/// A compact width shows the selected pane alone, the next one sliding in from the side it was swiped from (a cut with
/// Reduce Motion); a regular width shows the origin's split, or the selected pane alone once a pane tab asked for it
/// (`isAlone`). A machine whose ngterm can't stream says so, with
/// Detach.
internal struct SessionPanesView: View {
    @Environment(\.accessibilityReduceMotion)
    private var reduceMotion

    internal let session: AttachedSession
    internal let focused: String?
    internal let isCompact: Bool
    internal let actions: PaneSlot.Actions
    /// Whether the last move went forward: a compact width slides the next pane in from the trailing side.
    internal let isForward: Bool
    /// Whether a pane tab set the split aside.
    internal var isAlone = false

    internal var body: some View {
        let spots = actions.model.spots(of: session)
        if let layout = session.live.layout, !spots.isEmpty {
            if isCompact {
                onePane(spots)
            } else {
                SplitLayout(spots: spots, layout: layout, focused: focused, isSplit: !isAlone) { spot in
                    slot(spot, swipes: false)
                }
            }
        } else if session.live.connection == .unsupported {
            StatePanel.error(
                String(localized: "\(session.machine)'s ngterm can't share live sessions"),
                message: String(
                    localized: "Update ngterm on \(session.machine) to watch and type into its sessions here."
                ),
                action: RowAction(String(localized: "Detach")) {
                    actions.model.detach()
                }
            )
        } else {
            StatePanel.loading(String(localized: "Opening \(session.name)"))
        }
    }

    // a phone's one pane at a time
    private func onePane(_ spots: [PaneSpot]) -> some View {
        let shown = spots.first { $0.id == focused } ?? spots[0]
        let edge: Edge = isForward ? .trailing : .leading
        return ZStack {
            slot(shown, swipes: spots.count > 1)
                .padding(.horizontal, Spacing.xSmall)
                .id(shown.id)
                .transition(reduceMotion ? .identity : .push(from: edge))
        }
        .animation(Motion.animation(Motion.standard, reduceMotion: reduceMotion), value: shown.id)
    }

    private func slot(_ spot: PaneSpot, swipes: Bool) -> some View {
        PaneSlot(
            spot: spot,
            isFocused: spot.id == focused,
            actions: swipes ? actions : actions.withoutSwipe
        )
    }
}
