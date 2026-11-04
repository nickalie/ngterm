import NgtermDesign
import NgtermPlatform
import SwiftUI

/// Add machine, or Edit machine: the name, host, port and user, the key to sign in with (a choice only when the app
/// has more than one) and the way to its public key, and for a machine already added its pinned host key.
///
/// A field says what is wrong with it once it has been left or Save was tried; Save with problems only shows them.
internal struct MachineFormView: View {
    @Environment(\.dismiss)
    private var dismiss

    private let model: MachinesModel
    private let editing: StoredMachine?
    private let pin: String?
    @State private var draft: MachineDraft
    @State private var isTried = false
    @State private var left: Set<MachineField> = []
    @FocusState private var focus: MachineField?

    internal var body: some View {
        NavigationStack {
            Form {
                addressSection
                keySection
                if editing != nil {
                    pinSection
                }
            }
            .navigationTitle(editing == nil ? String(localized: "Add machine") : String(localized: "Edit machine"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(String(localized: "Cancel")) {
                        dismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "Save")) {
                        save()
                    }
                    .accessibilityIdentifier("machine-save")
                }
            }
            .scrollDismissesKeyboard(.immediately)
            .accessibilityIdentifier("machine-form")
            .onSubmit {
                // return on a one-line field closes the keyboard: the whole form is in view again
                focus = nil
            }
            .onChange(of: focus) { previous, _ in
                if let previous {
                    left.insert(previous)
                }
            }
        }
    }

    private var pinSection: some View {
        Section {
            if let pin {
                Text(verbatim: pin)
                    .font(Typography.address)
                    .textSelection(.enabled)
            } else {
                Text("None yet: the first connection asks you to confirm it.")
                    .foregroundStyle(Palette.secondaryText.color)
            }
        } header: {
            FormHint(String(localized: "Pinned host key"), .header)
        }
    }

    private var addressSection: some View {
        Section {
            FormField(
                field: .label,
                title: String(localized: "Name"),
                text: $draft.label,
                problem: nil,
                focus: $focus,
                wraps: true
            )
            .textInputAutocapitalization(.sentences)
            Group {
                FormField(
                    field: .host,
                    title: String(localized: "Host"),
                    text: $draft.host,
                    problem: problem(.host),
                    focus: $focus,
                    isMono: true,
                    wraps: true
                )
                .keyboardType(.URL)
                FormField(
                    field: .port,
                    title: String(localized: "Port"),
                    text: $draft.port,
                    problem: problem(.port),
                    focus: $focus,
                    isMono: true
                )
                .keyboardType(.numberPad)
                FormField(
                    field: .user,
                    title: String(localized: "User"),
                    text: $draft.user,
                    problem: problem(.user),
                    focus: $focus,
                    isMono: true
                )
            }
            .textInputAutocapitalization(.never)
        } footer: {
            FormHint(String(localized: "The name shows in the list; the host when it's empty."), .note)
        }
    }

    @ViewBuilder private var keySection: some View {
        let keys = model.keys
        Section {
            if keys.count > 1 {
                Picker(String(localized: "Key"), selection: $draft.keyId) {
                    ForEach(keys) { key in
                        KeyChoiceLabel(key: key)
                            .tag(key.id)
                    }
                }
                .pickerStyle(.navigationLink)
            } else if let key = keys.first {
                // the one key, said plainly: the system's labeled row greys its label under 4.5:1
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    Text("Key")
                    KeyChoiceLabel(key: key)
                }
                .accessibilityElement(children: .combine)
            }
            if let key = keys.first(where: { $0.id == draft.keyId }) {
                NavigationLink(String(localized: "Show public key")) {
                    PublicKeyView(key: key)
                }
            }
        } footer: {
            if let problem = problem(.key) {
                InlineProblem(problem.message)
            }
        }
    }

    /// The form for `target`: a new machine, or the one it edits; with `showsProblems`, as if Save was tried.
    internal init(model: MachinesModel, target: FormTarget, showsProblems: Bool) {
        self.model = model
        let edited: StoredMachine? =
            if case .edit(let machineId) = target { model.machine(id: machineId) } else { nil }
        editing = edited
        pin = edited.flatMap { model.pin(of: $0.id) }
        var initial = edited.map(MachineDraft.init(editing:)) ?? MachineDraft()
        if initial.keyId.isEmpty, let first = model.keys.first {
            initial.keyId = first.id
        }
        _draft = State(initialValue: initial)
        _isTried = State(initialValue: showsProblems)
    }

    // a field shows its problem from when it is left, or once Save was tried
    private func problem(_ field: MachineField) -> FieldProblem? {
        guard isTried || left.contains(field) else {
            return nil
        }
        return draft.problems[field]
    }

    private func save() {
        isTried = true
        if model.save(draft, editing: editing?.id) {
            dismiss()
        }
    }
}
