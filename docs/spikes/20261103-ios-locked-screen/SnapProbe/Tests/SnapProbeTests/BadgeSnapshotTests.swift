import SnapProbe
import SnapshotTesting
import SwiftUI
import XCTest

final class BadgeSnapshotTests: XCTestCase {
    @MainActor
    func testBadge() {
        let controller = UIHostingController(rootView: Badge())
        assertSnapshot(of: controller, as: .image(on: .iPhone13), record: .missing)
    }
}
