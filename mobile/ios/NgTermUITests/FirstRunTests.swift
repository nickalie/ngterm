import XCTest

/// The first run on the fake core: the welcome, the import and back, creating a key and its public half, then the
/// machines; each screen audited.
internal final class FirstRunTests: XCTestCase {
    @MainActor
    internal func testCreatingAKeyLeadsToTheMachines() throws {
        let app = FakeApp.launch(withKey: false, for: self)
        let create = app.buttons["create-key"]
        FakeApp.expect(create, "Create a key")
        try FakeApp.audit(app)

        app.buttons["import-key"].tap()
        FakeApp.expect(app.navigationBars["Import a key"], "import screen")
        try FakeApp.audit(app)
        app.buttons["import-key-action"].tap()
        let problem = FakeApp.element(containing: "Paste a private key or choose its file.", in: app)
        FakeApp.expect(problem, "empty import's problem")
        app.navigationBars["Import a key"].buttons.element(boundBy: 0).tap()

        FakeApp.expect(create, "the welcome again")
        create.tap()
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "public key screen")
        XCTAssertTrue(FakeApp.element(containing: "Add this key to your machines", in: app).exists)
        try FakeApp.audit(app)
        done.tap()

        FakeApp.showMachines(in: app)
        FakeApp.expect(FakeApp.element(containing: "No machines yet", in: app), "machines' empty state")
        try FakeApp.audit(app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
