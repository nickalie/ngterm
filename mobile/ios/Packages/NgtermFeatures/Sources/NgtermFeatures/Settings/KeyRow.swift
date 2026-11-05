import NgtermDesign
import SwiftUI

/// A key as Settings lists it: where it came from, its fingerprint, the machines that use it, and its public half's
/// actions, Show public key and Copy public key, side by side or one over the other when they don't fit.
internal struct KeyRow: View {
    internal let item: KeyItem
    internal let onShow: @MainActor () -> Void

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            VStack(alignment: .leading, spacing: Spacing.xSmall) {
                Text(
                    item.key.isImported
                        ? String(localized: "Imported") : String(localized: "Created on this device")
                )
                .font(.headline)
                Text(verbatim: item.key.fingerprint)
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
                    .textSelection(.enabled)
                Text(usage)
                    .font(Typography.detail)
                    .foregroundStyle(Palette.secondaryText.color)
            }
            .accessibilityElement(children: .combine)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: Spacing.large) {
                    actions
                }
                VStack(alignment: .leading, spacing: 0) {
                    actions
                }
            }
        }
        .padding(.vertical, Spacing.xSmall)
    }

    private var usage: String {
        item.usedBy.isEmpty
            ? String(localized: "No machine uses it")
            : String(localized: "Used by \(item.usedBy.formatted(.list(type: .and)))")
    }

    @ViewBuilder private var actions: some View {
        Button {
            onShow()
        } label: {
            Label(String(localized: "Show public key"), systemImage: "key")
                .font(Typography.detail.weight(.semibold))
                .frame(minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityIdentifier("show-key-\(item.key.id)")
        CopyButton(item.key.authorizedKey, title: String(localized: "Copy public key"))
    }
}
