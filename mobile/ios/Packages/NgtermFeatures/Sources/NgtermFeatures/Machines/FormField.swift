import NgtermDesign
import SwiftUI

/// A text field of the machine form with its title, and under it what is wrong with it, if anything.
internal struct FormField: View {
    internal let field: MachineField
    internal let title: String
    @Binding internal var text: String
    internal let problem: FieldProblem?
    internal var focus: FocusState<MachineField?>.Binding
    internal var isMono = false
    // a field that wraps (a name, a host) rather than scroll its text out of sight at large text sizes; Return types
    // a newline in it, which the draft trims from either end (as Android's trim()), and one inside a host reads as the
    // space problem
    internal var wraps = false

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            TextField(title, text: $text, axis: wraps ? .vertical : .horizontal)
                .font(isMono ? Typography.plateValue : .body)
                .autocorrectionDisabled()
                .frame(minHeight: Spacing.hitTarget)
                .focused(focus, equals: field)
                .accessibilityIdentifier("machine-\(field)")
            if let problem {
                InlineProblem(problem.message)
                    .padding(.bottom, Spacing.xSmall)
            }
        }
    }
}
