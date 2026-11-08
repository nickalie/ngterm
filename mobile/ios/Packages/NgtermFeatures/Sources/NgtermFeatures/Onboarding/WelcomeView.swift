import NgtermDesign
import SwiftUI

/// The first run's first screen, while the app has no key.
///
/// Create one (the main action) or import one. An import that goes on after Back says so here, and Create waits for it.
internal struct WelcomeView: View {
    internal let model: OnboardingModel

    internal var body: some View {
        let (isBusy, failure) = state
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                Image(systemName: "key.horizontal.fill")
                    .font(.largeTitle)
                    .imageScale(.large)
                    .foregroundStyle(Palette.tint.color)
                    .accessibilityHidden(true)
                Text("Set up a key")
                    .font(.largeTitle.bold())
                    .accessibilityAddTraits(.isHeader)
                Text(
                    """
                    ngterm signs in to your machines over ssh with a key that stays on this device. \
                    Create one, or import a key you already use.
                    """
                )
                .foregroundStyle(Palette.secondaryText.color)
                Label {
                    Text("The private key is kept in this device's Keychain and never leaves it.")
                } icon: {
                    Image(systemName: "lock.fill")
                        .foregroundStyle(Palette.healthy.color)
                }
                .font(.subheadline)
                .foregroundStyle(Palette.secondaryText.color)
                if let failure {
                    InlineProblem(String(localized: "The key couldn't be created: \(failure)"))
                }
                if model.isImporting {
                    importProgress
                }
                actions(isBusy: isBusy)
            }
            .frame(maxWidth: OnboardingLayout.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.xxLarge)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }

    private var state: (isBusy: Bool, failure: String?) {
        if case .welcome(let isBusy, let failure) = model.step {
            return (isBusy, failure)
        }
        return (false, nil)
    }

    // an import that goes on after Back: why Create waits
    private var importProgress: some View {
        HStack(spacing: Spacing.small) {
            ProgressView()
            Text("Importing a key")
        }
        .font(.subheadline)
        .foregroundStyle(Palette.secondaryText.color)
        .accessibilityElement(children: .combine)
    }

    private func actions(isBusy: Bool) -> some View {
        VStack(spacing: Spacing.medium) {
            BusyButton(
                title: isBusy ? String(localized: "Creating a key") : String(localized: "Create a key"),
                isBusy: isBusy
            ) {
                Task {
                    await model.create()
                }
            }
            .disabled(model.isImporting)
            .accessibilityIdentifier("create-key")
            Button {
                model.startImport()
            } label: {
                Text("Import a key")
                    .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .disabled(isBusy)
            .accessibilityIdentifier("import-key")
        }
        .padding(.top, Spacing.small)
    }
}
