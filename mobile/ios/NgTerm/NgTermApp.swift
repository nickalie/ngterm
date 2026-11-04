import SwiftUI

/// The iOS app: a remote-only viewer of ngterm machines' live sessions.
@main
internal struct NgTermApp: App {
    private let core = CoreChoice(arguments: ProcessInfo.processInfo.arguments)

    internal var body: some Scene {
        WindowGroup {
            RootView(coreVersion: core.version)
        }
    }
}
