import XCTest

/// Settings' Export private key on a real machine, as the UI tests open and drive it (``ExportKeyOnAMachineTests``,
/// ``KeyPassphraseOnAMachineTests``): past the system's owner check, under a passphrase.
internal enum ExportPage {
    // how many swipes a row far down the form may take to come into view
    private static let swipes = 12

    /// Waits `seconds`, with nothing to wait for.
    internal static func pause(_ seconds: TimeInterval) {
        _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: seconds)
    }

    // taps Settings' Export private key of the app's one key
    @MainActor
    private static func tapExport(in app: XCUIApplication) {
        let export = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'export-key-'")).firstMatch
        let form = app.collectionViews["settings-form"]
        FormScroll.scroll(to: export, in: form, swipes: swipes)
        export.tap()
    }

    /// Taps Export private key and shows the system's prompt a face or finger that doesn't match, then cancels it.
    @MainActor
    internal static func refuseOwnerCheck(in app: XCUIApplication) {
        OwnerCheck.refuse { tapExport(in: app) }
    }

    /// Taps Export private key and shows the system's prompt a matching face or finger, until the export opens.
    @MainActor
    internal static func passOwnerCheck(in app: XCUIApplication) {
        OwnerCheck.pass(by: { tapExport(in: app) }, in: app, until: app.navigationBars["Export private key"])
    }

    /// Types `passphrase` into both fields, each emptied first, and closes the keyboard after the second.
    @MainActor
    internal static func typePassphrase(_ passphrase: String, in app: XCUIApplication) {
        for identifier in ["export-passphrase", "export-passphrase-again"] {
            let field = app.secureTextFields[identifier]
            FakeApp.expect(field, identifier)
            SecureEntry.type(passphrase, into: field)
            if identifier.hasSuffix("again") {
                field.typeText("\n")
            }
        }
    }
}
