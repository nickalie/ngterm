import Foundation
import NgtermCore
import Testing

@testable import NgTerm

/// The hosted tests' smoke test: they run inside the app, on the core the app links.
@Suite("app host")
internal struct AppHostTests {
    @Test("the tests run inside the app")
    internal func hostIsTheApp() {
        #expect(Bundle.main.bundleIdentifier == "com.nickalie.ngterm.ios")
    }

    @Test("the app runs on the linked core unless the fake one is asked for")
    internal func coreChoice() {
        #expect(CoreChoice(arguments: []).version == CoreInfo.version)
        #expect(CoreChoice(arguments: [CoreChoice.fakeArgument]).version != CoreInfo.version)
    }
}
