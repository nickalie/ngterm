import NgtermCore
import NgtermDesign
import SwiftUI

/// One row of a pick (mobile/android's `PickRowView`).
///
/// An item's label over its subtitle with its status glyph at the end, as the sidebar draws it (none for an absent or
/// idle status), or the custom answer in the tint. The marked row, the one Return answers with, sits on a deeper
/// ground.
internal struct PickItemRow: View {
    private static let idle = "idle"

    @Environment(\.colorScheme)
    private var colorScheme
    @Environment(\.colorSchemeContrast)
    private var contrast

    internal let row: PickRow
    internal let items: [RemotePickItem]
    internal let isMarked: Bool
    internal let onPick: @MainActor () -> Void

    internal var body: some View {
        Button(action: onPick) {
            HStack(spacing: Spacing.medium) {
                content
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, Spacing.medium)
            .padding(.vertical, Spacing.small)
            .frame(minHeight: Spacing.hitTarget)
            .background(
                isMarked ? Color(uiColor: .tertiarySystemGroupedBackground) : Color.clear,
                in: RoundedRectangle(cornerRadius: Radius.small, style: .continuous)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder private var content: some View {
        switch row {
        case .custom(let label, _):
            Text(verbatim: label)
                .foregroundStyle(Palette.tint.color)
                .accessibilityIdentifier("pick-custom")

        case .item(let index):
            if Int(index) < items.count {
                item(items[Int(index)])
            }
        }
    }

    private func item(_ item: RemotePickItem) -> some View {
        let look = statusLook(of: item)
        return HStack(spacing: Spacing.medium) {
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(verbatim: item.label)
                    .foregroundStyle(.primary)
                    .lineLimit(Spacing.rowLines)
                if let subtitle = item.subtitle, !AskText.isBlank(subtitle) {
                    Text(verbatim: subtitle)
                        .font(.footnote)
                        .foregroundStyle(Palette.secondaryText.color)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if let look {
                StatusGlyph(look.shape, color: look.color.color)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(look?.word ?? "")
        .accessibilityIdentifier("pick-item-\(item.id)")
    }

    // the glyph the sidebar draws for the item's status: none for an absent or idle one
    private func statusLook(of item: RemotePickItem) -> AgentStatusLook? {
        guard let status = item.status, !AskText.isBlank(status), status != Self.idle else {
            return nil
        }
        return AgentStatusLook(
            status: status,
            colorHex: item.statusColor,
            shape: item.statusShape,
            in: Appearance(colorScheme: colorScheme, contrast: contrast)
        )
    }
}
