import NgtermDesign
import SwiftUI

/// What lies over a pane whose link dropped (mobile/android's `Reconnecting`).
///
/// The last screen stays under a light scrim that takes every touch, with a strip at the top saying the link is coming
/// back. VoiceOver hears the strip as it comes, and still reads the screen under it.
internal struct ReconnectingStrip: View {
    // light enough that the last screen reads through it, heavy enough to say it isn't live
    private static let scrim = 0.4

    internal let machine: String

    internal var body: some View {
        ZStack(alignment: .top) {
            Rectangle()
                .fill(Color(uiColor: .systemBackground).opacity(Self.scrim))
                .contentShape(Rectangle())
                .onTapGesture {
                    // the scrim takes the touch: the pane under it isn't live
                }
                .accessibilityHidden(true)
            HStack(spacing: Spacing.medium) {
                ProgressView()
                    .controlSize(.small)
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    Text(String(localized: "Reconnecting to \(machine)"))
                        .font(Typography.banner)
                    Text(String(localized: "The last screen stays until it is back."))
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, Spacing.large)
            .padding(.vertical, Spacing.medium)
            .background(
                Color(uiColor: .secondarySystemBackground),
                in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
            )
            .padding(Spacing.small)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("pane-reconnecting")
            .onAppear {
                AccessibilityNotification.Announcement(String(localized: "Reconnecting to \(machine)")).post()
            }
        }
    }
}
