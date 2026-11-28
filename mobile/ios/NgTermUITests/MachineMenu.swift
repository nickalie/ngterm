import XCUIAutomation

/// A machine's row menu, as the UI tests open it.
internal enum MachineMenu {
    // how long a row is held for its menu
    private static let press: TimeInterval = 1
    // how many swipes a row far down the form may take to come into view
    private static let swipes = 12

    /// Opens the form of the machine `label` from its row's menu, scrolled to its timeout field; returns the field.
    @MainActor
    @discardableResult
    internal static func edit(_ label: String, in app: XCUIApplication) -> XCUIElement {
        FakeApp.showMachines(in: app)
        let row = app.descendants(matching: .any)["machine-row-\(label)"].firstMatch
        FakeApp.expect(row, "the row of \(label)")
        row.press(forDuration: press)
        let edit = app.buttons["Edit"]
        FakeApp.expect(edit, "Edit in the menu of \(label)")
        edit.tap()
        let field = app.textFields["machine-timeout"]
        FakeApp.expect(field, "the timeout field of \(label)")
        FormScroll.scroll(to: field, in: app.collectionViews["machine-form"], swipes: swipes)
        return field
    }
}
