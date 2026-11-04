import NgtermDesign
import SwiftUI

/// One session's row: its name, its context (or its working directory) or the parent it is a companion of, and its
/// agent's status; "Ending" with a spinner while its End is under way.
///
/// A tap attaches it and opens it (the list's selection). Its context menu has Attach or Detach and End session; its
/// swipe actions, End session and, when attached, Detach, which VoiceOver offers as the row's actions. End session
/// only asks (`onAskEnd`): the screen confirms it.
internal struct SessionEntry: View {
    internal let line: SessionLine
    internal let onOpen: @MainActor () -> Void
    internal let onDetach: @MainActor () -> Void
    internal let onAskEnd: @MainActor () -> Void

    internal var body: some View {
        let session = line.session
        SessionRow(
            name: session.name,
            line: line.line,
            isCompanion: line.companionOf != nil,
            // the Attached group's heading says it: the mark would only crowd the row
            isAttached: false,
            isEnding: line.isEnding
        ) {
            if let status = session.status, !status.allSatisfy(\.isWhitespace) {
                AgentStatusBadge(status: status, colorHex: session.statusColor, shape: session.statusShape)
            }
        }
        .accessibilityIdentifier("session-\(session.name)")
        .contextMenu {
            if line.isAttached {
                Button(String(localized: "Detach"), systemImage: "rectangle.portrait.and.arrow.right", action: onDetach)
            } else {
                Button(String(localized: "Attach"), systemImage: "link", action: onOpen)
            }
            Button(String(localized: "End session"), systemImage: "xmark.circle", role: .destructive, action: onAskEnd)
                .disabled(line.isEnding)
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            // no destructive role: the row stays until the machine's listing lets it go, after the confirmation
            Button(String(localized: "End session"), systemImage: "xmark.circle", action: onAskEnd)
                .tint(Palette.failed.color)
                .disabled(line.isEnding)
            if line.isAttached {
                Button(String(localized: "Detach"), systemImage: "rectangle.portrait.and.arrow.right", action: onDetach)
                    .tint(Palette.idle.color)
            }
        }
    }
}
