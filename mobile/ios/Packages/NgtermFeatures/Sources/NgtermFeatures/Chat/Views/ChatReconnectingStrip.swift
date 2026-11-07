import NgtermDesign
import SwiftUI

/// The chat's link dropped and resumes on its own (mobile/android's `ReconnectingStrip`): a strip over the top of the
/// rows, which stay readable under it, with a spinner and the machine's name.
///
/// Announced as it comes.
internal struct ChatReconnectingStrip: View {
    internal let machine: String

    internal var body: some View {
        HStack(spacing: Spacing.medium) {
            ProgressView()
                .controlSize(.small)
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(String(localized: "Reconnecting to \(machine)"))
                    .font(Typography.banner)
                Text(String(localized: "What's here stays; new messages follow once it's back."))
                    .font(Typography.note)
                    .foregroundStyle(Palette.secondaryText.color)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, Spacing.large)
        .padding(.vertical, Spacing.medium)
        .background(ChatGrounds.raised, in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous))
        .frame(maxWidth: Spacing.readableWidth)
        .padding(Spacing.small)
        .accessibilityElement(children: .combine)
        .onAppear {
            AccessibilityNotification.Announcement(String(localized: "Reconnecting to \(machine)")).post()
        }
    }
}
