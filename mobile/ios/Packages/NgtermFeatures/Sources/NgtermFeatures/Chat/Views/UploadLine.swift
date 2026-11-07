import NgtermDesign
import SwiftUI

/// The attached file over the pill (mobile/android's `UploadLine`): on its way, its name, how much has gone and Cancel,
/// over a thin bar; every byte gone, "Saving" its name over the full bar, with no Cancel (the machine has it); stopped
/// short, why (in `failed`, announced) and Dismiss.
internal struct UploadLine: View {
    internal let upload: ChatUpload
    internal let onCancel: @MainActor () -> Void
    internal let onDismiss: @MainActor () -> Void

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xxSmall) {
            HStack(spacing: Spacing.small) {
                Image(systemName: ChatStyle.attachSymbol)
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)
                    .accessibilityHidden(true)
                content
            }
            .frame(minHeight: Spacing.hitTarget)
            if case .sending(_, let sent, let total, let isSaving) = upload {
                ProgressView(value: isSaving ? 1 : (total > 0 ? Double(sent) / Double(total) : 0))
                    .accessibilityHidden(true)
            }
        }
        .padding(.horizontal, Spacing.xSmall)
    }

    @ViewBuilder private var content: some View {
        switch upload {
        case .sending(let name, let sent, let total, let isSaving):
            Text(isSaving ? String(localized: "Saving \(name)") : String(localized: "Uploading \(name)"))
                .font(.footnote)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if !isSaving {
                Text("\(total > 0 ? Int(sent * 100 / total) : 0)%")
                    .font(.footnote.weight(.medium).monospacedDigit())
                    .foregroundStyle(Palette.secondaryText.color)
                closeButton(String(localized: "Cancel upload"), onCancel)
            }

        case .failed(let name, let problem):
            let why: String =
                switch problem {
                case .tooLarge:
                    String(localized: "it's over the 32 MB an upload takes")

                case .unreadable:
                    String(localized: "the device couldn't read it")

                case .refused(let reason):
                    reason
                }
            let words = String(localized: "Couldn't attach \(name): \(why)")
            Text(words)
                .font(.footnote)
                .foregroundStyle(Palette.failed.color)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .onAppear {
                    AccessibilityNotification.Announcement(words).post()
                }
            closeButton(String(localized: "Dismiss"), onDismiss)
        }
    }

    private func closeButton(_ label: String, _ action: @escaping @MainActor () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .font(.footnote.weight(.semibold))
                .frame(width: Spacing.hitTarget, height: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .foregroundStyle(Palette.secondaryText.color)
        .accessibilityLabel(label)
    }
}
