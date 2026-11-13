import SwiftUI

/// A machine's sessions as a list: the banners (the device is offline, the last refresh failed), the families with a
/// session this app has attached under "Attached", then the others under "On build-01", companions under their
/// parents, and under the last group a `note` when the machine's app lacks features (agterm).
///
/// Its selection is the session this window shows; selecting another attaches and opens it (`onOpen`). A row's
/// Duplicate session asks `onDuplicate` for a copy of it.
internal struct SessionsList: View {
    internal let machine: String
    internal let attached: [SessionLine]
    internal let others: [SessionLine]
    internal let banners: SessionsBanners
    internal var note: String?
    internal let selection: String?
    internal let onOpen: @MainActor (String) -> Void
    internal let onDetach: @MainActor (String) -> Void
    internal let onDuplicate: @MainActor (String) -> Void
    internal let chatView: @MainActor (String) -> ChatViewChoice
    internal let onAskEnd: @MainActor (SessionLine) -> Void

    internal var body: some View {
        List(selection: selected) {
            if banners.isShown {
                Section {
                    banners
                        .listRowInsets(EdgeInsets())
                        .listRowBackground(Color.clear)
                }
            }
            group(String(localized: "Attached"), attached, note: others.isEmpty ? note : nil)
            group(String(localized: "On \(machine)"), others, note: note)
        }
        .listStyle(.insetGrouped)
        .accessibilityIdentifier("sessions")
    }

    private var selected: Binding<String?> {
        Binding {
            selection
        } set: { sessionId in
            if let sessionId {
                onOpen(sessionId)
            }
        }
    }

    @ViewBuilder
    private func group(_ title: String, _ lines: [SessionLine], note: String?) -> some View {
        if !lines.isEmpty {
            Section {
                ForEach(lines) { line in
                    SessionEntry(line: line, chatView: chatView(line.id)) {
                        onOpen(line.id)
                    } onDetach: {
                        onDetach(line.id)
                    } onDuplicate: {
                        onDuplicate(line.id)
                    } onAskEnd: {
                        onAskEnd(line)
                    }
                    .tag(line.id)
                }
            } header: {
                FormHint(title, .header)
            } footer: {
                if let note {
                    FormHint(note, .note)
                        .accessibilityIdentifier("sessions-peer-note")
                }
            }
        }
    }
}
