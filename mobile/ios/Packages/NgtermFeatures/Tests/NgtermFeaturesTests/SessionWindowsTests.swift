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
        let model = ScaffoldModel(windows: windows)
        #expect(model.open(docs) == .here)
        windows.opened(docs)
        #expect(model.open(docs) == .ownWindow)
        #expect(model.open(api) == .here)
        windows.closed(docs)
        #expect(model.open(docs) == .here)
    }

    @Test("a session shows in one main window at a time: opening it in another takes it over, and back")
    internal func mainWindowsTakeASessionOver() {
        let windows = SessionWindows()
        let first = ScaffoldModel(windows: windows)
        let second = ScaffoldModel(windows: windows)
        _ = first.open(docs)
        #expect(second.open(docs) == .here)
        #expect(second.shownSessionPlacement == .here)
        #expect(first.shownSessionPlacement == .otherWindow)
        #expect(first.session == docs, "the first window keeps what it showed, and can take it back")

        #expect(first.open(docs) == .here)
        #expect(first.shownSessionPlacement == .here)
        #expect(second.shownSessionPlacement == .otherWindow)
    }

    @Test("a main window leaving a session, or going away, frees it for the others")
    internal func leavingASessionFreesIt() {
        let windows = SessionWindows()
        let other = ScaffoldModel(windows: windows)
        _ = other.open(docs)
        var leaving: ScaffoldModel? = ScaffoldModel(windows: windows)
        _ = leaving?.open(docs)
        #expect(other.shownSessionPlacement == .otherWindow)
        _ = leaving?.open(api)
        #expect(other.shownSessionPlacement == .here, "switching to another session frees the first")

        _ = other.open(api)
        #expect(leaving?.shownSessionPlacement == .otherWindow)
        _ = leaving?.open(api)
        leaving?.show(machine: "nas")
        #expect(other.shownSessionPlacement == .here, "showing another machine frees the session")

        _ = leaving?.open(api)
        #expect(other.shownSessionPlacement == .otherWindow)
        leaving = nil
        #expect(other.shownSessionPlacement == .here, "a scene that goes away frees its session")
    }

    @Test("with three main windows, a session freed by its holder goes back to one of the others only")
    internal func leavingASessionFreesItForOneWindow() {
        let windows = SessionWindows()
        let first = ScaffoldModel(windows: windows)
        let second = ScaffoldModel(windows: windows)
        var third: ScaffoldModel? = ScaffoldModel(windows: windows)
        _ = first.open(docs)
        _ = second.open(docs)
        _ = third?.open(docs)
        #expect(first.shownSessionPlacement == .otherWindow)
        #expect(second.shownSessionPlacement == .otherWindow)
        #expect(third?.shownSessionPlacement == .here)

        _ = third?.open(api)
        #expect(second.shownSessionPlacement == .here, "the window that had it last takes it back")
        #expect(first.shownSessionPlacement == .otherWindow, "and only that one")

        _ = third?.open(docs)
        third?.show(machine: "nas")
        #expect(second.shownSessionPlacement == .here, "showing another machine frees it for one window")
        #expect(first.shownSessionPlacement == .otherWindow)

        _ = third?.open(docs)
        third = nil
        #expect(second.shownSessionPlacement == .here, "a scene that goes away frees it for one window")
        #expect(first.shownSessionPlacement == .otherWindow)

        _ = first.open(docs)
        #expect(first.shownSessionPlacement == .here, "a waiting window takes it over")
        #expect(second.shownSessionPlacement == .otherWindow)
        _ = first.open(api)
        #expect(second.shownSessionPlacement == .here)
        _ = second.open(api)
        #expect(first.shownSessionPlacement == .otherWindow)
        #expect(second.shownSessionPlacement == .here)
    }

    @Test("a session whose own window closes goes back to one main window only")
    internal func closingTheOwnWindowHandsItToOneMainWindow() {
        let windows = SessionWindows()
        let first = ScaffoldModel(windows: windows)
        let second = ScaffoldModel(windows: windows)
        let third = ScaffoldModel(windows: windows)
        _ = first.open(docs)
        _ = second.open(docs)
        _ = third.open(docs)
        windows.opened(docs)
        #expect([first, second, third].allSatisfy { $0.shownSessionPlacement == .ownWindow })
        windows.closed(docs)
        #expect(third.shownSessionPlacement == .here)
        #expect(first.shownSessionPlacement == .otherWindow)
        #expect(second.shownSessionPlacement == .otherWindow)
    }

    @Test("a session with its own window shows in no main window, whichever had it")
    internal func ownWindowWinsOverMainWindows() {
        let windows = SessionWindows()
        let first = ScaffoldModel(windows: windows)
        let second = ScaffoldModel(windows: windows)
        _ = first.open(docs)
        windows.opened(docs)
        #expect(first.shownSessionPlacement == .ownWindow)
        #expect(second.open(docs) == .ownWindow)
        #expect(second.session == nil)
    }

    @Test("Show machines is offered only while the machines' column is hidden")
    internal func showMachinesOnlyWhenHidden() {
        let model = ScaffoldModel(windows: SessionWindows())
        #expect(!model.areMachinesHidden, "all three columns where they fit")
        model.columnVisibility = .doubleColumn
        #expect(model.areMachinesHidden)
        model.columnVisibility = .detailOnly
        #expect(model.areMachinesHidden)
    }

    @Test("a removed machine's sessions and session leave the window, and a compact stack goes back to the machines")
    internal func forgetMachine() {
        let windows = SessionWindows()
        let model = ScaffoldModel(windows: windows)
        let other = ScaffoldModel(windows: windows)
        _ = other.open(docs)
        _ = model.open(docs)
        model.forget(machine: "nas")
        #expect(model.session == docs)
        #expect(other.shownSessionPlacement == .otherWindow)
        model.forget(machine: "build-01")
        #expect(model.machine == nil)
        #expect(model.session == nil)
        #expect(model.compactColumn == .sidebar)
        // the window let the session go: the one that had it before shows it again
        #expect(other.shownSessionPlacement == .here)
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
