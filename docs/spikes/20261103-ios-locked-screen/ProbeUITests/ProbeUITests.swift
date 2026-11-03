import XCTest

final class ProbeUITests: XCTestCase {
    @MainActor
    func testTapCounts() throws {
        let app = XCUIApplication()
        app.launch()
        app.buttons["tap"].tap()
        app.buttons["tap"].tap()
        XCTAssertEqual(app.staticTexts["count"].label, "taps: 2")
    }
}
