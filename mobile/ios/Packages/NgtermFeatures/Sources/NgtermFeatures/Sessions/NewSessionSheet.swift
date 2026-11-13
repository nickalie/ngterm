import NgtermDesign
import SwiftUI

/// New session on a machine: an optional name (blank lets the machine name it) and Create.
///
/// While the create runs (until its session is attached and opened) Create gives way to a spinner and the field
/// holds still; Cancel closes the sheet and the create goes on, its session only listed. What went wrong shows under
/// the field, which keeps its text for another try.
internal struct NewSessionSheet: View {
    internal let machine: String
    internal let state: NewSessionState
    // another create (a Duplicate, another window's New session) holds Create until it ends
    internal var isHeld = false
    internal let onCreate: @MainActor (String) -> Void
    internal let onCancel: @MainActor () -> Void
    @State private var name = ""

    internal var body: some View {
        NavigationStack {
            Form {
                Section {
                    // built as the machine form's fields are (FormField), at least 44 pt tall; not focused on its
                    // own: a field being edited reports only its text's height, under 44 pt
                    VStack {
                        TextField(String(localized: "Name"), text: $name, prompt: Text("Optional"), axis: .horizontal)
                            .font(Typography.address)
                            .frame(minHeight: Spacing.hitTarget)
                    }
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .disabled(state.isRunning)
                    .onSubmit(create)
                    .accessibilityIdentifier("session-name")
                } header: {
                    FormHint(String(localized: "Name"), .header)
                } footer: {
                    if let failure = state.failure {
                        InlineProblem(failure)
                    } else {
                        FormHint(String(localized: "Optional: \(machine) names it when blank."), .note)
                    }
                }
            }
            .navigationTitle(String(localized: "New session"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "Cancel"), action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    if state.isRunning {
                        ProgressView()
                            .accessibilityLabel(String(localized: "Creating"))
                    } else {
                        Button(String(localized: "Create"), action: create)
                            .disabled(!state.takesCreate(isHeld: isHeld))
                            .accessibilityIdentifier("create-session")
                    }
                }
            }
        }
    }

    private func create() {
        if state.takesCreate(isHeld: isHeld) {
            onCreate(name)
        }
    }
}
