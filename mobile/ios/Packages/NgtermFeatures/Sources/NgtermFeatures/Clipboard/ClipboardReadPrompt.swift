import NgtermDesign
import NgtermTerminal
import SwiftUI

/// Asks about the first clipboard read the shown panes' programs wait on, in a sheet over whatever shows.
///
/// The sheet (mobile/android's `ClipboardReadPrompt`) names the machine and the session the read comes from, and
/// offers Allow, Always allow on the machine, or Deny. Swiping it away denies; a sheet whose read went (answered
/// elsewhere, or dismissed by the core) goes by itself.
internal struct ClipboardReadPrompt: ViewModifier {
    internal let graph: AppGraph
    @State private var shown: ClipboardRead?

    // a sheet the user swiped away denies its read; one closed because its read went is left as it is
    private var presented: Binding<ClipboardRead?> {
        Binding {
            shown
        } set: { read in
            if read == nil, let gone = shown, graph.clipboardReads.pending.contains(gone) {
                graph.clipboardReads.deny(gone)
            }
            shown = read
        }
    }

    internal func body(content: Content) -> some View {
        content
            .onChange(of: graph.clipboardReads.pending.first, initial: true) { _, read in
                shown = read
            }
            .sheet(item: presented) { read in
                question(for: read)
                    .ngtermTheme()
            }
    }

    private func question(for read: ClipboardRead) -> some View {
        let pane = read.pane
        let reads = graph.clipboardReads
        let machine = graph.machines.machine(id: pane.machineId)?.title ?? pane.machineId
        let listed = graph.connections.trees[pane.machineId]?.first { $0.id == pane.sessionId }
        let session = listed?.name ?? pane.sessionId
        return ClipboardReadQuestion(listOnly: read.listOnly, machine: machine, session: session) {
            reads.allow(read)
        } onAlways: {
            reads.allowAlways(read)
        } onDeny: {
            reads.deny(read)
        }
    }
}

extension View {
    /// Asks about the clipboard reads of the panes `graph` shows, in a sheet over this view.
    public func clipboardReadPrompt(_ graph: AppGraph) -> some View {
        modifier(ClipboardReadPrompt(graph: graph))
    }
}
