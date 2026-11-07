import NgtermCore
import NgtermDesign
import NgtermTerminal
import SwiftUI

/// A pane's open chat, live, read from `machine` (mobile/android's `ChatPane` and `ChatScreen`).
///
/// Its rows, or what stands in for them (the backlog on its way, nothing said yet, the chat ended), a strip over the
/// rows while its link is on its way back, pinned at its foot what the agent still has running (unless a card waits),
/// the last action's problem and the card the agent waits on (an approval before a question), and under them the
/// composer, whose `/` menu opens over the foot of the rows (the activity line and its Stop under it), or over the card
/// while one waits. `onShowTerminal` shows the pane's terminal instead. VoiceOver names it "Chat with" the agent.
///
/// Its prompt field's focus follows the selection (`isSelected`): a chat the selection leaves lets it go, and one it
/// moves to takes it when the keyboard was in use (`keyboard`'s handover). Only the selected chat takes the
/// keyboard's chords: ⌘Return, and the ⌘⌥ arrows that move between panes and sessions.
internal struct ChatScreen: View {
    // the most of the chat's height a card takes before it scrolls inside: the rows above it stay in sight
    private static let cardShare: CGFloat = 0.6

    internal let chat: ChatModel
    internal let machine: String
    internal let keyboard: TerminalKeyboard
    internal let isSelected: Bool
    internal let onShowTerminal: @MainActor () -> Void
    @State private var open = OpenRows()
    // swiftlint:disable:next unused_declaration - read only through its binding, which the activity line toggles
    @State private var isActivityOpen = false
    @FocusState private var isFieldFocused: Bool

    internal var body: some View {
        let snapshot = chat.snapshot
        let agent = snapshot.conversation?.agent
        GeometryReader { geometry in
            VStack(spacing: 0) {
                if let account = snapshot.account, snapshot.phase != .ended {
                    AccountLine(account: account)
                        .frame(maxWidth: Spacing.readableWidth)
                        .padding(.horizontal, Spacing.large)
                }
                ZStack(alignment: .bottom) {
                    VStack(spacing: 0) {
                        ZStack(alignment: .bottom) {
                            content(snapshot, agent: agent)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                            if !snapshot.awaitsCard {
                                menu(agent: agent)
                            }
                        }
                        .overlay(alignment: .top) {
                            if snapshot.phase == .reconnecting {
                                ChatReconnectingStrip(machine: machine)
                            }
                        }
                        if snapshot.phase != .ended {
                            pinned(snapshot, agent: agent, cardHeight: geometry.size.height * Self.cardShare)
                        }
                    }
                    if snapshot.awaitsCard {
                        menu(agent: agent)
                    }
                }
                if snapshot.phase != .ended {
                    ChatComposerView(
                        composer: chat.composer,
                        snapshot: snapshot,
                        actions: actions,
                        isSelected: isSelected,
                        isFieldFocused: $isFieldFocused
                    )
                }
            }
        }
        .background(ChatGrounds.chat)
        .background {
            if isSelected {
                ChatChords(keyboard: keyboard)
            }
        }
        .onChange(of: isFieldFocused) { _, isFocused in
            fieldFocused(isFocused, field: "prompt")
        }
        .onChange(of: isSelected, initial: true) { _, selected in
            if !selected {
                isFieldFocused = false
            } else if keyboard.takeChatHandover() {
                isFieldFocused = true
            }
        }
        .onChange(of: snapshot.conversation?.id) { before, now in
            // another conversation's rows open nothing here
            if before != nil, now != before {
                open = OpenRows(conversation: now)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(String(localized: "Chat with \(ChatStyle.agentName(agent))"))
        .accessibilityIdentifier("chat")
    }

    private var actions: ChatActions {
        let model = chat
        return ChatActions(
            onShowTerminal: onShowTerminal,
            onSend: { model.send() },
            onAttach: { picked in model.attach(name: picked.name, size: picked.size, read: picked.read) },
            onCancelUpload: { model.cancelUpload() },
            onUploadShown: { model.uploadShown() }
        )
    }

    // the keyboard knows which chat's field has it (each field its own key, so one taking the focus from the other in
    // this chat isn't undone by the other letting it go), and a move of the selection hands it on
    private func fieldFocused(_ isFocused: Bool, field: String) {
        let key = AnyHashable([AnyHashable(chat.pane), AnyHashable(field)])
        if isFocused {
            keyboard.chatField = key
        } else if keyboard.chatField == key {
            keyboard.chatField = nil
        }
    }

    @ViewBuilder
    private func content(_ snapshot: ChatSnapshot, agent: String?) -> some View {
        let name = ChatStyle.agentName(agent)
        let isWaiting = snapshot.phase == .loading || snapshot.phase == .reconnecting
        let showTerminal = RowAction(String(localized: "Show terminal"), perform: onShowTerminal)
        if snapshot.phase == .ended {
            StatePanel.empty(
                systemImage: ChatStyle.chatSymbol,
                title: String(localized: "This chat ended"),
                message: String(
                    localized: """
                        \(name) isn't running in this pane any more, or \(machine) can't share its conversation. The \
                        terminal shows the pane as it is.
                        """
                ),
                action: showTerminal
            )
        } else if snapshot.log.isEmpty, !snapshot.log.switched, isWaiting {
            ChatLoading(machine: machine)
        } else if snapshot.log.isEmpty, !snapshot.log.switched {
            StatePanel.empty(
                systemImage: ChatStyle.chatSymbol,
                title: String(localized: "Nothing said yet"),
                message: String(
                    localized: "\(name)'s replies, the tools it runs and its questions show here as it works."
                ),
                action: showTerminal
            )
        } else {
            let model = chat
            ChatList(
                snapshot: snapshot,
                onLoadOlder: { model.loadOlder() },
                onEndShown: { model.endShown() },
                open: $open
            )
        }
    }

    // the `/` menu while it is open, over what is above the composer
    @ViewBuilder
    private func menu(agent: String?) -> some View {
        if let menu = chat.composer.menu {
            let composer = chat.composer
            SlashCommandMenu(menu: menu, agent: agent) { command in
                composer.pick(command)
                // the field keeps the focus, and the keyboard with it
                isFieldFocused = true
            }
            .frame(maxWidth: Spacing.readableWidth)
            .padding(.horizontal, Spacing.small)
            .padding(.vertical, Spacing.xSmall)
        }
    }

    // what waits at the chat's foot: what still runs (while no card waits), the last action's problem, then the card
    private func pinned(_ snapshot: ChatSnapshot, agent: String?, cardHeight: CGFloat) -> some View {
        VStack(spacing: Spacing.small) {
            if !snapshot.awaitsCard {
                activity(snapshot, agent: agent)
            }
            if let problem = snapshot.problem {
                problemBanner(problem)
            }
            card(snapshot, agent: agent, cardHeight: cardHeight)
        }
        .frame(maxWidth: Spacing.readableWidth)
        .frame(maxWidth: .infinity)
    }

    private func activity(_ snapshot: ChatSnapshot, agent: String?) -> some View {
        let model = chat
        let stop: (@MainActor () -> Void)? =
            if model.isRunning {
                { @MainActor in model.stop() }
            } else {
                nil
            }
        return ActivityStrip(
            isWorking: model.isWorking,
            running: snapshot.log.running,
            agent: ChatStyle.agentName(agent),
            onStop: stop,
            isStopEnabled: !snapshot.isBusy,
            isExpanded: $isActivityOpen
        )
        .padding(.horizontal, Spacing.small)
    }

    private func problemBanner(_ problem: ChatProblem) -> some View {
        let model = chat
        return ChatProblemBanner(problem: problem, onShowTerminal: onShowTerminal) {
            model.problemShown()
        }
        .padding(.horizontal, Spacing.small)
    }

    // the card the agent waits on, an approval before a question; a tap answers the card it was drawn on, whatever
    // replaced it before the tap landed
    @ViewBuilder
    private func card(_ snapshot: ChatSnapshot, agent: String?, cardHeight: CGFloat) -> some View {
        let model = chat
        if let approval = snapshot.approval {
            let answer: @MainActor (String) -> Void = { label in
                model.answerApproval(id: approval.id, label: label)
            }
            ApprovalCardView(
                card: approval,
                agent: agent,
                isBusy: snapshot.isBusy,
                maxHeight: cardHeight,
                onAnswer: answer
            )
        } else if let question = snapshot.question {
            let answer: @MainActor ([QuestionAnswer]) -> Void = { answers in
                model.answerQuestion(id: question.id, answers: answers)
            }
            QuestionCardView(
                card: question,
                agent: agent,
                isBusy: snapshot.isBusy,
                maxHeight: cardHeight,
                onAnswer: answer,
                selection: QuestionCardView.Selection(isSelected: isSelected) { isFocused in
                    fieldFocused(isFocused, field: "answer")
                }
            )
        }
    }
}
