import SwiftUI

/// A strip about the whole screen while it stays usable: "You're offline", "Couldn't refresh".
///
/// The status glyph, the message and its note, an optional action and an optional dismiss. The text wraps, and the
/// actions go under it when they don't fit beside it. VoiceOver announces it when it appears, without moving the focus.
public struct Banner: View {
    private let tone: StatusTone
    private let message: String
    private let note: String?
    private let action: RowAction?
    private let dismiss: RowAction?

    /// The body of the view.
    public var body: some View {
        // the actions go under the text when the row can't hold them beside it (a long message, a large text size)
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
                glyph
                text
                actions
            }
            HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
                glyph
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    text
                    if action != nil || dismiss != nil {
                        HStack(spacing: Spacing.small) {
                            actions
                        }
                    }
                }
            }
        }
        .padding(.horizontal, Spacing.medium)
        .padding(.vertical, Spacing.xSmall)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
        .onAppear {
            AccessibilityNotification.Announcement(message).post()
        }
    }

    private var glyph: some View {
        StatusGlyph(tone.shape, color: tone.color.color)
    }

    // the message and its note, on as many lines as they take
    private var text: some View {
        VStack(alignment: .leading, spacing: Spacing.xxSmall) {
            Text(message)
                .font(Typography.banner)
                .fixedSize(horizontal: false, vertical: true)
            if let note {
                Text(note)
                    .font(Typography.note)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private var actions: some View {
        if let action {
            QuietButton(action: action)
                .font(Typography.banner)
        }
        if let dismiss {
            Button(action: dismiss.perform) {
                Image(systemName: "xmark")
                    .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .accessibilityLabel(dismiss.title)
        }
    }

    /// A banner in `tone` saying `message` and `note`, with its `action`, that stays until what it says changes.
    public init(_ tone: StatusTone, message: String, note: String? = nil, action: RowAction? = nil) {
        self.tone = tone
        self.message = message
        self.note = note
        self.action = action
        self.dismiss = nil
    }

    /// The banner with a close button doing `dismiss`, read by VoiceOver as "Dismiss".
    @preconcurrency
    public init(
        _ tone: StatusTone,
        message: String,
        note: String? = nil,
        action: RowAction? = nil,
        dismiss: @escaping @MainActor () -> Void
    ) {
        self.tone = tone
        self.message = message
        self.note = note
        self.action = action
        self.dismiss = RowAction(String(localized: "Dismiss", bundle: .module), perform: dismiss)
    }
}
