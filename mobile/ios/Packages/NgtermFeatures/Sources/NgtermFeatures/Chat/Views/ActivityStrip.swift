import NgtermCore
import NgtermDesign
import SwiftUI

/// What the agent still has on its way (mobile/android's `ActivityStrip`), as one quiet line just over the composer.
///
/// So a finished reply isn't read as the work being done: "Claude Code is working" while its status says it works on a
/// turn (`isWorking`), else "Still running in the background" while calls of the conversation have no result yet
/// (`running`: a subagent or a command its turn left going), and after it how many of each; with any running a tap
/// lists them under it, each as its tool line. While the turn can be interrupted (`onStop` given) Stop, a word in the
/// tint, ends the line, held while an action is on its way. Nothing to say shows nothing. The ring stands still:
/// nothing at the foot moves on its own.
internal struct ActivityStrip: View {
    private static let ringPoints: CGFloat = 12

    internal let isWorking: Bool
    internal let running: [ChatTool]
    internal let agent: String
    internal let onStop: (@MainActor () -> Void)?
    internal let isStopEnabled: Bool
    @Binding internal var isExpanded: Bool

    internal var body: some View {
        if isWorking || !running.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 0) {
                    label
                    if let onStop {
                        Button(String(localized: "Stop"), action: onStop)
                            .font(.subheadline.weight(.semibold))
                            .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                            .disabled(!isStopEnabled)
                            .accessibilityIdentifier("chat-stop")
                    }
                }
                if isExpanded, !running.isEmpty {
                    VStack(alignment: .leading, spacing: Spacing.xSmall) {
                        ForEach(Array(running.enumerated()), id: \.offset) { _, tool in
                            ChatLine(
                                symbol: ChatStyle.symbol(tool.kind),
                                glyphLabel: ChatStyle.word(tool.kind),
                                title: ChatStyle.title(tool)
                            ) {
                                EmptyView()
                            }
                        }
                    }
                    .padding(.leading, Spacing.medium)
                }
            }
        }
    }

    // the line itself: the still ring, what the agent is doing and how many run; with any running the whole of it is a
    // 44 pt target that lists them, and VoiceOver reads whether the list is open
    @ViewBuilder private var label: some View {
        let title =
            isWorking
            ? String(localized: "\(agent) is working") : String(localized: "Still running in the background")
        let row = HStack(spacing: Spacing.small) {
            Image(systemName: StatusShape.ring.symbolName)
                .font(.system(size: Self.ringPoints, weight: .semibold))
                .foregroundStyle(Palette.connecting.color)
                .accessibilityHidden(true)
            Text(title)
                .font(.subheadline.weight(.medium))
                .foregroundStyle(Palette.secondaryText.color)
                .lineLimit(1)
                .layoutPriority(1)
            if !running.isEmpty {
                Text(Self.counts(running))
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
                    .lineLimit(1)
            }
        }
        .padding(.leading, Spacing.medium)
        .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget, alignment: .leading)
        .contentShape(Rectangle())
        if running.isEmpty {
            row
                .accessibilityElement(children: .combine)
        } else {
            Button {
                isExpanded.toggle()
            } label: {
                row
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)
            .accessibilityValue(isExpanded ? String(localized: "Expanded") : String(localized: "Collapsed"))
            .accessibilityHint(
                isExpanded ? String(localized: "Hides what's running") : String(localized: "Shows what's running")
            )
        }
    }

    /// How many subagents, commands and other tools run, in that order, the ones with none left out.
    internal static func counts(_ running: [ChatTool]) -> String {
        let subagents = running.count { $0.kind == .task }
        let commands = running.count { $0.kind == .shell }
        let tools = running.count - subagents - commands
        var parts: [String] = []
        if subagents > 0 {
            parts.append(subagents == 1 ? String(localized: "1 subagent") : String(localized: "\(subagents) subagents"))
        }
        if commands > 0 {
            parts.append(commands == 1 ? String(localized: "1 command") : String(localized: "\(commands) commands"))
        }
        if tools > 0 {
            parts.append(tools == 1 ? String(localized: "1 tool") : String(localized: "\(tools) tools"))
        }
        return parts.joined(separator: ", ")
    }
}
