import NgtermDesign
import SwiftUI

/// The chat's foot (mobile/android's `Composer`), in a messenger's shape: one rounded field on the bar, its buttons in
/// it.
///
/// Attach, the paperclip, at its start, and at its end one place that Commands, a `/` that starts one, gives up to
/// Send, the return key's arrow in the tint, once there is something to send; the prompt between them grows to six
/// lines, then scrolls inside. Stop is the activity line's, over it. A file attached shows over the pill on its way,
/// with Cancel, or why it stopped; its stored path goes into the prompt once it is there. While a card waits a line
/// over the field says to answer it first, and Send holds; the field still takes a draft. While an action is on its way
/// Send shows a spinner, and the field keeps the keyboard: a prompt sent leaves it empty for the next one at once.
///
/// A hardware keyboard: ⌘Return sends; while the `/` menu lists commands, Up and Down move its highlight and Return or
/// Tab picks it; Escape closes the menu. Return alone is a line break.
internal struct ChatComposerView: View {
    private static let pillHeight: CGFloat = 44
    private static let pillRadius: CGFloat = 22
    private static let maxLines = 6
    // how small the leaving button shrinks to, and the arriving one starts from
    private static let swapScale: CGFloat = 0.6
    private static let slashPoints: CGFloat = 18

    @Environment(\.accessibilityReduceMotion)
    private var reduceMotion

    @Bindable internal var composer: ChatComposer
    internal let snapshot: ChatSnapshot
    internal let actions: ChatActions
    internal let isSelected: Bool
    @FocusState.Binding internal var isFieldFocused: Bool

    internal var body: some View {
        let hasText = !AskText.isBlank(composer.text)
        let isSending = snapshot.isBusy && !snapshot.awaitsCard
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            if snapshot.awaitsCard {
                Text(String(localized: "Answer the card above first: a prompt now would go to its dialog."))
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
                    .padding(.horizontal, Spacing.xSmall)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let upload = snapshot.upload {
                UploadLine(upload: upload, onCancel: actions.onCancelUpload, onDismiss: actions.onUploadShown)
            }
            HStack(alignment: .bottom, spacing: 0) {
                AttachButton(isEnabled: snapshot.upload?.isSending != true, onPick: actions.onAttach)
                field
                endSlot(showsSend: hasText || isSending, isSending: isSending)
            }
            .frame(minHeight: Self.pillHeight)
            .background(ChatGrounds.raised, in: RoundedRectangle(cornerRadius: Self.pillRadius, style: .continuous))
        }
        .frame(maxWidth: Spacing.readableWidth)
        .padding(.horizontal, Spacing.small)
        .padding(.vertical, Spacing.small)
        .frame(maxWidth: .infinity)
        .background(.bar)
    }

    private var canSend: Bool {
        !snapshot.isBusy && !snapshot.awaitsCard && !AskText.isBlank(composer.text)
    }

    private var field: some View {
        TextField(
            String(localized: "Prompt, or / for commands"),
            text: Binding {
                composer.text
            } set: { typed in
                composer.edit(typed)
            },
            selection: $composer.selection,
            axis: .vertical
        )
        .lineLimit(1...Self.maxLines)
        .font(.body)
        .textInputAutocapitalization(.sentences)
        .focused($isFieldFocused)
        .padding(.vertical, Spacing.medium)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityIdentifier("chat-prompt")
        .onKeyPress(keys: [.upArrow, .downArrow, .return, .tab, .escape]) { press in
            key(press)
        }
    }

    private var commandsButton: some View {
        Button {
            composer.startCommand()
            isFieldFocused = true
        } label: {
            Text(verbatim: "/")
                .font(Fonts.terminal(size: Self.slashPoints, relativeTo: .body))
                .frame(width: Spacing.hitTarget, height: Self.pillHeight)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(Palette.secondaryText.color)
        .disabled(snapshot.isBusy)
        .accessibilityLabel(String(localized: "Commands"))
        .accessibilityIdentifier("chat-commands")
    }

    private var swap: AnyTransition {
        reduceMotion ? .identity : .scale(scale: Self.swapScale).combined(with: .opacity)
    }

    // the pill's last place, Commands' or Send's: the one leaving shrinks and fades as the other grows in from its
    // middle, both 44 pt, so the prompt never changes width as it is typed
    private func endSlot(showsSend: Bool, isSending: Bool) -> some View {
        ZStack {
            if showsSend {
                sendButton(isSending: isSending)
                    .transition(swap)
            } else {
                commandsButton
                    .transition(swap)
            }
        }
        .frame(width: Spacing.hitTarget, height: Self.pillHeight)
        .animation(Motion.animation(Motion.quick, reduceMotion: reduceMotion), value: showsSend)
    }

    private func sendButton(isSending: Bool) -> some View {
        Button(action: actions.onSend) {
            Group {
                if isSending {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: ChatStyle.sendSymbol)
                        .font(.body.weight(.semibold))
                }
            }
            .frame(width: Spacing.hitTarget, height: Self.pillHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(canSend ? Palette.tint.color : Palette.secondaryText.color)
        .disabled(!canSend)
        // the shortcut is the selected chat's alone: two chats on screen would both claim it otherwise
        .keyboardShortcut(isSelected ? KeyboardShortcut(.return, modifiers: .command) : nil)
        .accessibilityLabel(isSending ? String(localized: "Sending") : String(localized: "Send"))
        .accessibilityIdentifier("chat-send")
    }

    // a hardware key in the field: the menu's keys while it lists commands, Escape while it is open
    private func key(_ press: KeyPress) -> KeyPress.Result {
        let isHandled: Bool =
            switch press.key {
            case .downArrow:
                composer.moveHighlight(by: 1)

            case .upArrow:
                composer.moveHighlight(by: -1)

            case .return where press.modifiers.isEmpty, .tab:
                composer.pickHighlighted()

            case .escape:
                composer.dismissMenu()

            default:
                false
            }
        return isHandled ? .handled : .ignored
    }
}
