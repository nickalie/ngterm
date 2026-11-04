import Testing

@testable import NgtermFeatures

/// The one-window-per-session rule, and a window's navigation over it.
@MainActor
@Suite("session windows")
internal struct SessionWindowsTests {
    private let docs = SessionWindowID(machineID: "build-01", sessionID: "A1")
    private let api = SessionWindowID(machineID: "build-01", sessionID: "B2")

    @Test("a session opens here until it has a window of its own, and again once that closes")
    internal func placementFollowsTheOwnWindows() {
        let windows = SessionWindows()
        #expect(windows.placement(of: docs) == .here)
        windows.opened(docs)
        #expect(windows.placement(of: docs) == .ownWindow)
        #expect(windows.placement(of: api) == .here)
        windows.closed(docs)
        #expect(windows.placement(of: docs) == .here)
    }

    @Test("opening a session shows it here and moves a compact stack to it")
    internal func openingShowsTheSessionHere() {
        let model = ScaffoldModel(windows: SessionWindows())
        #expect(model.open(docs) == .here)
        #expect(model.session == docs)
        #expect(model.machine == "build-01")
        #expect(model.compactColumn == .detail)
    }

    @Test("a session with its own window doesn't open here: the caller brings that window forward")
    internal func openingASessionWithItsOwnWindow() {
        let windows = SessionWindows()
        let model = ScaffoldModel(windows: windows)
        #expect(model.open(api) == .here)
        windows.opened(docs)
        #expect(model.open(docs) == .ownWindow)
        #expect(model.session == api, "the window keeps what it showed")
    }

    @Test("a session shown here steps aside when it gets a window of its own")
    internal func shownSessionStepsAside() {
        let windows = SessionWindows()
        let model = ScaffoldModel(windows: windows)
        #expect(model.shownSessionPlacement == nil)
        _ = model.open(docs)
        #expect(model.shownSessionPlacement == .here)
        windows.opened(docs)
        #expect(model.shownSessionPlacement == .ownWindow)
    }

    @Test("showing another machine leaves the session of the last one")
    internal func showingAMachine() {
        let model = ScaffoldModel(windows: SessionWindows())
        _ = model.open(docs)
        model.show(machine: "build-01")
        #expect(model.session == docs)
        model.show(machine: "nas")
        #expect(model.session == nil)
        #expect(model.machine == "nas")
        #expect(model.compactColumn == .content)
    }

    @Test("the onboarding covers the first run only")
    internal func onboardingHook() {
        #expect(ScaffoldModel(windows: SessionWindows(), needsOnboarding: true).isOnboardingPresented)
        #expect(!ScaffoldModel(windows: SessionWindows()).isOnboardingPresented)
    }
}
