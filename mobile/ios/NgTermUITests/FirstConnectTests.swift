import XCTest

/// A first connect on the fake core.
///
/// Add a machine, confirm its host key, and it connects; and a machine that asks to sign in. Each sheet is audited.
internal final class FirstConnectTests: XCTestCase {
    @MainActor
    internal func testAddMachineConfirmFingerprintConnected() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        try FakeApp.addMachine(host: "build-01.lan", user: "nick", in: app, audit: true)

        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "host key sheet")
        XCTAssertTrue(FakeApp.element(containing: "Trust build-01.lan?", in: app).exists)
        let fingerprint = "SHA256:fAkEbU1Ld01h0sTkEyF0rUiTeStS0nLyN0tArEaLkEy00"
        XCTAssertTrue(FakeApp.element(containing: fingerprint, in: app).exists)
        try FakeApp.auditSheet(app)
        trust.tap()

        FakeApp.expectRow("build-01.lan", reads: "Connected", in: app)
        XCTAssertFalse(trust.exists)
        try FakeApp.audit(app)
    }

    @MainActor
    internal func testTheSignInSheetWaitsAndDontConnectCancels() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        try FakeApp.addMachine(host: "signin.lan", user: "nick", in: app, audit: false)
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "host key sheet")
        trust.tap()

        let open = app.buttons["open-sign-in"]
        FakeApp.expect(open, "sign-in sheet")
        XCTAssertTrue(FakeApp.element(containing: "signin.lan asks you to sign in", in: app).exists)
        XCTAssertTrue(app.buttons["Copy link"].exists)
        XCTAssertTrue(FakeApp.element(containing: "https://login.tailscale.com/a/l1nk", in: app).exists)
        XCTAssertTrue(FakeApp.element(containing: "Waiting for you to sign in", in: app).exists)
        try FakeApp.auditSheet(app)
        app.buttons["cancel-sign-in"].tap()

        FakeApp.expectRow("signin.lan", reads: "Offline", in: app)
        XCTAssertFalse(open.exists)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
