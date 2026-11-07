import NgtermDesign
import SwiftUI

/// The permission prompt the agent waits on (mobile/android's `ApprovalCardView`): "Allow Bash?", the call's title in
/// the terminal's face, its detail (a command, or a diff) on a plate that scrolls past 160 pt, then the dialog's
/// options in its order, the first (the dialog's own default) filled.
///
/// Until the pane's screen is read the agent's own answers stand in, worded as its dialogs word them. While an answer
/// is on its way they hold still.
internal struct ApprovalCardView: View {
    private static let detailHeight: CGFloat = 160

    internal let card: ApprovalCard
    internal let agent: String?
    internal let isBusy: Bool
    internal let maxHeight: CGFloat
    internal let onAnswer: @MainActor (String) -> Void

    internal var body: some View {
        let approval = card.approval
        let labels = card.labels.isEmpty ? ChatStyle.genericApprovalLabels(agent) : card.labels
        NeedsYouCard(title: String(localized: "Allow \(approval.tool)?"), maxHeight: maxHeight) {
            if !AskText.isBlank(approval.title) {
                Text(verbatim: approval.title)
                    .font(Typography.chatCode)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let detail = approval.detail, !AskText.isBlank(detail), detail != approval.title {
                ScrollView(.vertical) {
                    if Self.looksLikeDiff(detail) {
                        DiffView(diff: detail)
                    } else {
                        OutputPlate(text: detail)
                    }
                }
                .frame(maxHeight: Self.detailHeight)
                .fixedSize(horizontal: false, vertical: true)
            }
            CardButtons(labels: labels, isBusy: isBusy, onPress: onAnswer)
        }
        .id(card.id)
    }

    private static func looksLikeDiff(_ text: String) -> Bool {
        text.hasPrefix("diff ") || text.hasPrefix("--- ") || text.hasPrefix("@@")
    }
}
