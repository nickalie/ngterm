import NgtermDesign
import SwiftUI

/// Why the last action did nothing, or not all it meant to (mobile/android's `ProblemBanner`).
///
/// It offers Answer in terminal when the terminal is the way on, in the attention tone when the user can act on it,
/// the failed one when it only reports.
internal struct ChatProblemBanner: View {
    internal let problem: ChatProblem
    internal let onShowTerminal: @MainActor () -> Void
    internal let onDismiss: @MainActor () -> Void

    internal var body: some View {
        Banner(
            problem.isActionable ? .attention : .failed,
            message: Self.message(problem),
            action: problem.offersTerminal
                ? RowAction(String(localized: "Answer in terminal"), perform: onShowTerminal) : nil,
            dismiss: onDismiss
        )
    }

    internal static func message(_ problem: ChatProblem) -> String {
        switch problem {
        case .notOnScreen:
            String(
                localized: """
                    Couldn't confirm the pane shows this, so nothing was sent. Use the terminal, or wait if it was \
                    answered elsewhere.
                    """
            )

        case .dialogOpen:
            String(localized: "A dialog is open in the pane. Answer it first: nothing was sent.")

        case .leadRefused:
            String(localized: "Another device kept control of this pane, so nothing was sent.")

        case .unconfirmed:
            String(localized: "The keys went, but the pane didn't show what they should have done. Check the terminal.")

        case .notReading:
            String(localized: "The program isn't reading its input. Nothing was sent; try again in a moment.")

        case .closed:
            String(localized: "This pane isn't on screen any more, so nothing was sent.")

        case .refused(let reason):
            String(localized: "Not sent: \(reason)")
        }
    }
}
