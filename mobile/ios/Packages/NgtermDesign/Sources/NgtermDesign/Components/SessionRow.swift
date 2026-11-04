import SwiftUI

/// A session in a list: its name with the Attached mark, its line, and its status.
///
/// The line is the context, the working directory, or "Companion of …"; the status an ``AgentStatusBadge``, or
/// "Ending" with a spinner while the session is being ended. A companion sits indented under its parent. It reads no
/// app state; its list adds the selection, swipe actions and the context menu.
public struct SessionRow<Status: View>: View {
    private let name: String
    private let line: String?
    private let isCompanion: Bool
    private let isAttached: Bool
    private let isEnding: Bool
    private let status: Status

    /// The body of the view.
    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
                Text(name)
                    .font(Typography.sessionName)
                    .lineLimit(Spacing.rowLines)
                if isAttached {
                    AttachedMark()
                }
            }
            if let line {
                Text(line)
                    .font(Typography.detail)
                    .foregroundStyle(Palette.secondaryText.color)
                    .lineLimit(Spacing.rowLines)
            }
            if isEnding {
                HStack(spacing: Spacing.xSmall) {
                    ProgressView()
                        .controlSize(.mini)
                    Text(String(localized: "Ending", bundle: .module))
                        .font(Typography.statusWord)
                        .foregroundStyle(Palette.secondaryText.color)
                }
                .accessibilityElement(children: .combine)
            } else {
                status
            }
        }
        .padding(.leading, isCompanion ? Spacing.xLarge : 0)
        .padding(.vertical, Spacing.xSmall)
        .frame(minHeight: Spacing.hitTarget)
        .accessibilityElement(children: .combine)
    }

    /// A session named `name` with its `line`, a companion or not, attached by this app or not, being ended or not,
    /// and its `status`.
    public init(
        name: String,
        line: String?,
        isCompanion: Bool = false,
        isAttached: Bool = false,
        isEnding: Bool = false,
        @ViewBuilder status: () -> Status
    ) {
        self.name = name
        self.line = line
        self.isCompanion = isCompanion
        self.isAttached = isAttached
        self.isEnding = isEnding
        self.status = status()
    }
}
