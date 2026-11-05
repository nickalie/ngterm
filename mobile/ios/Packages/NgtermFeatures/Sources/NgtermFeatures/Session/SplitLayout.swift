import NgtermCore
import NgtermDesign
import SwiftUI

/// A session's split as its origin lays it out (mobile/android's `SplitLayout`).
///
/// Two panes side by side (`vertical`, the divider's direction) or stacked (`horizontal`), split by a hairline, each
/// with a gutter at its leading edge where the selected one shows the focus bar. One pane, or a split the origin hides
/// (`shown` false), shows the selected pane alone.
internal struct SplitLayout<Pane: View>: View {
    internal let spots: [PaneSpot]
    internal let layout: RemoteLayout
    internal let focused: String?
    @ViewBuilder internal let pane: (PaneSpot) -> Pane

    internal var body: some View {
        if spots.count == SplitMetrics.panes, layout.shown, let axis = layout.axis {
            if axis == SplitMetrics.sideBySide {
                HStack(spacing: 0) {
                    half(spots[0])
                    Divider()
                    half(spots[1])
                }
            } else {
                VStack(spacing: 0) {
                    half(spots[0])
                    Divider()
                    half(spots[1])
                }
            }
        } else if let alone = spots.first(where: { $0.id == focused }) ?? spots.first {
            // a gutter as a split's, so the text keeps off the screen's edge
            pane(alone)
                .padding(.horizontal, Spacing.xSmall)
        }
    }

    // a pane of a shown split: the gutter with the focus bar when it has the focus, then the pane
    private func half(_ spot: PaneSpot) -> some View {
        HStack(spacing: 0) {
            Capsule()
                .fill(spot.id == focused ? Palette.tint.color : .clear)
                .frame(width: SplitMetrics.barWidth, height: SplitMetrics.barLength)
                .frame(width: Spacing.xSmall)
                .accessibilityHidden(true)
            pane(spot)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
