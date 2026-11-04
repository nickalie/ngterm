import NgtermDesign
import SwiftUI

/// Asks the questions the machines' connects wait on, one sheet at a time.
///
/// A host key to trust comes first, then a sign-in. A sheet goes once its question is answered (here, or by the
/// connect ending); swiping it away answers Don't connect.
internal struct ConnectPrompts: ViewModifier {
    internal let model: MachinesModel
    @State private var shown: ConnectPrompt?

    // a sheet the user swiped away answers Don't connect; one closed because its question went is left as it is
    // (MachinesModel.dismiss checks the connect still waits on that very question)
    private var presented: Binding<ConnectPrompt?> {
        Binding {
            shown
        } set: { prompt in
            if prompt == nil, let gone = shown {
                model.dismiss(gone)
            }
            shown = prompt
        }
    }

    internal func body(content: Content) -> some View {
        content
            .onChange(of: model.prompt, initial: true) { _, prompt in
                shown = prompt
            }
            .sheet(item: presented) { prompt in
                sheet(for: prompt)
                    .ngtermTheme()
            }
    }

    @ViewBuilder
    private func sheet(for prompt: ConnectPrompt) -> some View {
        switch prompt {
        case .hostKey(let machine, let fingerprint):
            HostKeySheet(machine: machine, fingerprint: fingerprint) { trust in
                model.answerHostKey(machineId: machine.id, fingerprint: fingerprint, trust: trust)
            }

        case .signIn(let machine, let message, let url):
            SignInSheet(machine: machine, message: message, url: url) {
                model.disconnect(machineId: machine.id)
            }
        }
    }
}

extension View {
    /// Asks the questions the connects of `model` wait on, in sheets over this view.
    public func connectPrompts(_ model: MachinesModel) -> some View {
        modifier(ConnectPrompts(model: model))
    }
}
