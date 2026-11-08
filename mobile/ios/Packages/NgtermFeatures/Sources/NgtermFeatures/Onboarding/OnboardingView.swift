import NgtermDesign
import SwiftUI

/// The first run, over everything (there is nowhere to go without a key): the welcome, the import pushed over it, and
/// the public key once there is one, whose Done ends the first run with `onFinish`; or why the app's keys couldn't be
/// read, with Retry, which ends it with `onFinish` too once they are read and there is one.
public struct OnboardingView: View {
    private let model: OnboardingModel
    private let onFinish: @MainActor () -> Void

    /// The body of the view.
    public var body: some View {
        NavigationStack {
            switch model.step {
            case .keyReady(let key):
                PublicKeyView(key: key) {
                    model.finish()
                    onFinish()
                }

            case .done:
                Color(uiColor: .systemGroupedBackground)

            case .readFailed(let reason):
                readFailed(reason: reason)

            case .importing, .welcome:
                WelcomeView(model: model)
                    .navigationDestination(isPresented: isImporting) {
                        ImportKeyView(work: importWork)
                    }
            }
        }
        .ngtermTheme()
    }

    // the import over the first run's model
    private var importWork: ImportKeyWork {
        let onboarding = model
        return ImportKeyWork {
            if case .importing(let isBusy, let problem) = onboarding.step {
                return (isBusy, problem)
            }
            return (false, nil)
        } importKey: { bytes, passphrase in
            await onboarding.importKey(privateKey: &bytes, passphrase: passphrase)
        } unreadable: {
            onboarding.unreadable()
        }
    }

    // the import is pushed over the welcome: Back pops it
    private var isImporting: Binding<Bool> {
        Binding {
            if case .importing = model.step {
                return true
            }
            return false
        } set: { isShown in
            if isShown {
                model.startImport()
            } else {
                model.back()
            }
        }
    }

    /// The first run of `model`; `onFinish` closes it once the key's public half has been seen.
    @preconcurrency
    public init(model: OnboardingModel, onFinish: @escaping @MainActor () -> Void) {
        self.model = model
        self.onFinish = onFinish
    }

    // why the keys couldn't be read (the system's words, which end their sentence), with Retry, which ends the first
    // run once they are read and there is a key
    private func readFailed(reason: String) -> some View {
        let words = reason.hasSuffix(".") ? String(reason.dropLast()) : reason
        return StatePanel.error(
            String(localized: "Your keys couldn't be read"),
            message: String(
                localized: """
                    The app's storage answered: \(words). \
                    Try again; if it keeps failing, restart the app.
                    """
            ),
            action: RowAction(String(localized: "Retry")) {
                model.retry()
                if model.step == .done {
                    onFinish()
                }
            }
        )
        .background(Color(uiColor: .systemGroupedBackground))
    }
}
