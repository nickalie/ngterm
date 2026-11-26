import XCTest

/// Import a key on the fake core brings no offer of the system's to keep the passphrase in Passwords.
///
/// The offer comes neither while an imported key's public half shows nor once that page is closed, nor when the
/// import is left with a refused passphrase in its field: by Back on the first run and in Settings, or with Settings
/// swiped away (``ExportKeyTests`` holds the export's side). The import empties the passphrase field while it runs,
/// and a refused passphrase comes back into it.
internal final class KeyPassphraseOfferTests: XCTestCase {
    // a passphrase the test key does not open with
    private static let wrong = "not-the-passphrase"
    // how many swipes Settings' Import a key may take to come into view
    private static let swipes = 12

    @MainActor
    internal func testAKeyImportedWithItsPassphraseBringsNoOfferToKeepIt() {
        // an offer takes the next tap: the steps after it are not what they say
        continueAfterFailure = false
        let app = FakeApp.launch(withKey: false, for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(TestKey.privateKey)
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(TestKey.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        FakeApp.expect(page, "the imported key's public half")
        PasswordOffer.expectNone(in: app, "after the import")

        app.buttons["key-done"].tap()
        FakeApp.showMachines(in: app)
        FakeApp.expect(FakeApp.element(containing: "No machines yet", in: app), "the machines")
        PasswordOffer.expectNone(in: app, "once the public key page closed")
    }

    // the field is empty while the import runs: a refused passphrase comes back into it, for another try
    @MainActor
    internal func testARefusedPassphraseStaysInItsField() {
        let app = FakeApp.launch(withKey: false, for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key")
        start.tap()
        refuse(in: app)
    }

    // Back from the first run's import, with a refused passphrase in its field, brings no offer
    @MainActor
    internal func testBackFromTheFirstRunsImportBringsNoOfferToKeepThePassphrase() {
        continueAfterFailure = false
        let app = FakeApp.launch(withKey: false, for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key")
        start.tap()
        refuse(in: app)
        app.navigationBars["Import a key"].buttons.element(boundBy: 0).tap()
        FakeApp.expect(start, "the welcome again")
        PasswordOffer.expectNone(in: app, "after Back from the import")
    }

    // Back from Settings' import, with a refused passphrase in its field, brings no offer
    @MainActor
    internal func testBackFromSettingsImportBringsNoOfferToKeepThePassphrase() {
        continueAfterFailure = false
        let app = openSettingsImport()
        refuse(in: app)
        app.navigationBars["Import a key"].buttons.element(boundBy: 0).tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings again")
        PasswordOffer.expectNone(in: app, "after Back from Settings' import")
    }

    // Settings swiped away from its import, with a refused passphrase in its field, brings no offer
    @MainActor
    internal func testSettingsSwipedAwayFromItsImportBringsNoOfferToKeepThePassphrase() {
        continueAfterFailure = false
        let app = openSettingsImport()
        refuse(in: app)
        app.navigationBars["Import a key"].swipeDown(velocity: .fast)
        FakeApp.expect(app.buttons["add-machine"], "the machines, Settings gone")
        PasswordOffer.expectNone(in: app, "after Settings was swiped away from its import")
    }

    // the app past its first run, Settings' Import a key open
    @MainActor
    private func openSettingsImport() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, for: self)
        FakeApp.showMachines(in: app)
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        let start = app.buttons["import-key"]
        let form = app.collectionViews["settings-form"]
        FormScroll.scroll(to: start, in: form, swipes: Self.swipes)
        start.tap()
        FakeApp.expect(app.navigationBars["Import a key"], "Settings' import")
        return app
    }

    // on the import screen: the key typed, a wrong passphrase typed and refused, so it stays in its field
    @MainActor
    private func refuse(in app: XCUIApplication) {
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(TestKey.privateKey)
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(Self.wrong + "\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(
            FakeApp.element(containing: "That passphrase doesn't open this key.", in: app),
            "that the wrong passphrase was refused"
        )
        XCTAssertEqual((secret.value as? String)?.count, Self.wrong.count, "the refused passphrase is not in its field")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
