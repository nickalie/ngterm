import NgtermDesign
import NgtermFeatures
import SwiftUI

/// The iOS app: a remote-only viewer of ngterm machines' live sessions.
@main
internal struct NgTermApp: App {
    #if DEBUG
        private let arguments = ProcessInfo.processInfo.arguments
    #endif

    @UIApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var windows = SessionWindows()

    private var core: CoreChoice {
        delegate.core
    }

    internal var body: some Scene {
        WindowGroup {
            #if DEBUG
                if arguments.contains(CoreChoice.fakeArgument), arguments.contains(CoreChoice.fakeTerminalArgument) {
                    TerminalBench()
                } else {
                    RootView(graph: core.graph, coreVersion: core.version, windows: windows)
                }
            #else
                RootView(graph: core.graph, coreVersion: core.version, windows: windows)
            #endif
        }
        // one window per session: SwiftUI keeps a window per value, and opening a session that has one focuses it
        WindowGroup(for: SessionWindowID.self) { session in
            if let id = session.wrappedValue {
                SessionWindow(graph: core.graph, session: id, windows: windows)
            }
        }
    }

    internal init() {
        Fonts.register()
    }
}
