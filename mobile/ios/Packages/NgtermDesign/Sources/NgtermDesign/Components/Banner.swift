import SwiftUI

/// A strip about the whole screen while it stays usable: "You're offline", "Couldn't refresh".
///
/// The status glyph, the message and its note, an optional action and an optional dismiss. VoiceOver announces it when
/// it appears, without moving the focus.
public struct Banner: View {
    private let tone: StatusTone
    private let message: String
    private let note: String?
    private let action: RowAction?
    private let dismiss: RowAction?

    /// The body of the view.
    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
            StatusGlyph(tone.shape, color: tone.color.color)
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                Text(message)
                    .font(Typography.banner)
                if let note {
                    Text(note)
                        .font(Typography.note)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            if let action {
                Button(action.title, action: action.perform)
                    .font(Typography.banner)
                    .buttonStyle(.borderless)
                    .frame(minHeight: Spacing.hitTarget)
            }
            if let dismiss {
                Button(action: dismiss.perform) {
                    Image(systemName: "xmark")
                        .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.secondary)
                .accessibilityLabel(dismiss.title)
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
