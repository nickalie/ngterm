import NgtermDesign
import SwiftUI

/// "Sending" with a small spinner, announced as it comes.
internal struct ChatSending: View {
    internal var body: some View {
        HStack(spacing: Spacing.small) {
            ProgressView()
                .controlSize(.small)
            Text(String(localized: "Sending"))
                .font(.subheadline)
                .foregroundStyle(Palette.secondaryText.color)
        }
        .accessibilityElement(children: .combine)
        .onAppear {
            AccessibilityNotification.Announcement(String(localized: "Sending")).post()
        }
    }
}
