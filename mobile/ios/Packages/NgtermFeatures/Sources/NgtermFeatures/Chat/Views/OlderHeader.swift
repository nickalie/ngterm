import NgtermDesign
import SwiftUI

/// The top of the list (mobile/android's `OlderHeader`): earlier messages on their way, the conversation's start with
/// its working directory, or why the last page failed with Retry.
internal struct OlderHeader: View {
    internal let older: OlderPages
    internal let cwd: String?
    internal let onRetry: @MainActor () -> Void

    internal var body: some View {
        HStack(spacing: Spacing.small) {
            switch older {
            case .more, .loading:
                ProgressView()
                    .controlSize(.small)
                Text(String(localized: "Loading earlier messages"))
                    .font(.footnote)
                    .foregroundStyle(Palette.secondaryText.color)

            case .atStart:
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    Text(String(localized: "Start of the conversation"))
                        .font(.footnote)
                    if let cwd {
                        Text(verbatim: cwd)
                            .font(Typography.address)
                    }
                }
                .foregroundStyle(Palette.secondaryText.color)
                .frame(maxWidth: .infinity, alignment: .leading)

            case .failed(let reason):
                Text(String(localized: "Couldn't load earlier messages: \(reason)"))
                    .font(.footnote)
                    .foregroundStyle(Palette.failed.color)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(String(localized: "Retry"), action: onRetry)
                    .frame(minHeight: Spacing.hitTarget)
            }
        }
        .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget, alignment: .leading)
        .padding(.horizontal, Spacing.xSmall)
        .accessibilityElement(children: .combine)
    }
}
