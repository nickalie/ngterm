import Foundation
import NgtermPlatform
import Testing

/// The app's `Info.plist` lets the background refresh run: `fetch` among its background modes, and the refresh's
/// identifier permitted to the scheduler.
@MainActor
@Suite("background modes")
internal struct BackgroundModesTests {
    @Test("Background App Refresh is declared, with the refresh's identifier permitted")
    internal func refresh() throws {
        let modes = try #require(Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String])
        #expect(modes == ["fetch"])
        let permitted = try #require(
            Bundle.main.object(forInfoDictionaryKey: "BGTaskSchedulerPermittedIdentifiers") as? [String]
        )
        #expect(permitted == [BackgroundRefresh.identifier])
    }
}
