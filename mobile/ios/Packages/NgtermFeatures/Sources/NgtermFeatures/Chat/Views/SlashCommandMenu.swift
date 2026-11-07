import NgtermCore
import NgtermDesign
import SwiftUI

/// The `/` menu over the composer (mobile/android's `SlashCommandMenu`): the commands matching what follows the `/`,
/// best first, about six in sight and the rest a scroll away, each row the command as it goes in (`/name`, its argument
/// hint after it), a line of what it does and where it comes from; the highlighted row (Return or Tab picks it) on a
/// deeper fill.
///
/// Under them a quiet line says how many match, or that none does, or that the agent's commands are on their way or not
/// listed (never an error: the prompt still goes as typed), announced as it changes. VoiceOver reads each row whole and
/// names its tap Insert.
internal struct SlashCommandMenu: View {
    private static let rowHeight: CGFloat = 56
    private static let visibleRows: CGFloat = 6

    internal let menu: SlashMenu
    internal let agent: String?
    internal let onPick: @MainActor (AgentCommand) -> Void

    internal var body: some View {
        VStack(spacing: 0) {
            if case .commands(_, let commands, let highlighted) = menu, !commands.isEmpty {
                ScrollViewReader { reader in
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(commands.enumerated()), id: \.offset) { index, command in
                                row(command, isHighlighted: index == highlighted)
                                    .id(index)
                            }
                        }
                    }
                    .frame(maxHeight: Self.rowHeight * Self.visibleRows)
                    .fixedSize(horizontal: false, vertical: true)
                    .onChange(of: highlighted) { _, index in
                        reader.scrollTo(index)
                    }
                    .onChange(of: menu.query) {
                        reader.scrollTo(0, anchor: .top)
                    }
                }
            }
            line
        }
        .background(ChatGrounds.raised, in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous))
        .accessibilityIdentifier("chat-command-menu")
    }

    @ViewBuilder private var line: some View {
        let name = ChatStyle.agentName(agent)
        let words: String =
            switch menu {
            case .commands(let query, let commands, _) where commands.isEmpty:
                String(localized: "No command matches /\(query). It goes as typed.")

            case .commands(_, let commands, _):
                Self.count(commands.count)

            case .waiting(_, true):
                String(localized: "Loading \(name)'s commands")

            case .waiting:
                String(localized: "\(name)'s commands aren't listed here. Type the command out.")
            }
        HStack(spacing: Spacing.small) {
            if case .waiting(_, true) = menu {
                ProgressView()
                    .controlSize(.small)
            }
            Text(words)
                .font(.footnote)
                .foregroundStyle(Palette.secondaryText.color)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, Spacing.large)
        .padding(.vertical, Spacing.small)
        .frame(minHeight: Spacing.hitTarget)
        .accessibilityElement(children: .combine)
        .onChange(of: words) { _, now in
            AccessibilityNotification.Announcement(now).post()
        }
    }

    // how many commands match: "1 command", "2 commands"
    private static func count(_ count: Int) -> String {
        if count == 1 {
            return String(localized: "1 command")
        }
        return String(localized: "\(count) commands")
    }

    private func row(_ command: AgentCommand, isHighlighted: Bool) -> some View {
        Button {
            onPick(command)
        } label: {
            HStack(spacing: Spacing.medium) {
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    HStack(spacing: Spacing.small) {
                        Text(verbatim: ChatComposer.insertion(command))
                            .font(Typography.chatCode)
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        if let hint = command.argumentHint, !AskText.isBlank(hint) {
                            Text(verbatim: hint)
                                .font(Typography.chatCode)
                                .foregroundStyle(Palette.secondaryText.color)
                                .lineLimit(1)
                        }
                    }
                    if !AskText.isBlank(command.description) {
                        Text(verbatim: command.description)
                            .font(.footnote)
                            .foregroundStyle(Palette.secondaryText.color)
                            .lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Text(ChatStyle.word(command.kind))
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(Palette.secondaryText.color)
            }
            .padding(.horizontal, Spacing.large)
            .padding(.vertical, Spacing.small)
            .frame(minHeight: Self.rowHeight)
            .background(isHighlighted ? ChatGrounds.highlighted : Color.clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isHighlighted ? .isSelected : [])
        .accessibilityHint(String(localized: "Inserts the command"))
    }
}
