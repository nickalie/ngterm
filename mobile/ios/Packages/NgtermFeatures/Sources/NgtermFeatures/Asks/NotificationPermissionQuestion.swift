import NgtermDesign
import SwiftUI

/// Why the app wants to notify, a sheet's page (mobile/android's `NotificationPermissionQuestion`): what reaches the
/// user (questions, an agent waiting or finished), that a question can be answered from the notification, and when
/// they come (while the app is open, for a short time after it, and from a refresh now and then); then **Turn on
/// notifications** (prominent) and **Not now** (quiet).
public struct NotificationPermissionQuestion: View {
    private static let width: Double = 520

    private let onAllow: @MainActor () -> Void
    private let onLater: @MainActor () -> Void

    /// The page.
    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                Image(systemName: "bell.badge")
                    .font(.title2)
                    .foregroundStyle(Palette.tint.color)
                    .accessibilityHidden(true)
                Text("Get told when an agent needs you")
                    .font(.title2.bold())
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                Text(
                    """
                    A notification brings you back when an agent asks you something, waits for you or finishes. You \
                    can answer a question from the notification itself.
                    """
                )
                .foregroundStyle(Palette.secondaryText.color)
                .fixedSize(horizontal: false, vertical: true)
                Text(
                    """
                    They come while ngterm is open and for a short time after you leave it. Later, iOS lets ngterm \
                    look in on your attached sessions now and then, and tells you when an agent waits or finishes.
                    """
                )
                .foregroundStyle(Palette.secondaryText.color)
                .fixedSize(horizontal: false, vertical: true)
                answers
                    .padding(.top, Spacing.small)
            }
            .frame(maxWidth: Self.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }

    private var answers: some View {
        VStack(spacing: Spacing.small) {
            Button {
                onAllow()
            } label: {
                Text("Turn on notifications")
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            .accessibilityIdentifier("notifications-allow")
            Button {
                onLater()
            } label: {
                Text("Not now")
                    .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .accessibilityIdentifier("notifications-later")
        }
    }

    /// The question; **Turn on notifications** runs `onAllow`, **Not now** `onLater`.
    @preconcurrency
    public init(onAllow: @escaping @MainActor () -> Void, onLater: @escaping @MainActor () -> Void) {
        self.onAllow = onAllow
        self.onLater = onLater
    }
}
