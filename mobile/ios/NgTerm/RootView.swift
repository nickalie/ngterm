import SwiftUI

/// The app's first screen until its features land: the name and the core's version.
internal struct RootView: View {
    internal let coreVersion: String

    internal var body: some View {
        VStack {
            Text(verbatim: "ngterm")
                .font(.largeTitle)
            Text(verbatim: "core \(coreVersion)")
                .font(.footnote.monospaced())
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("core-version")
        }
    }
}
