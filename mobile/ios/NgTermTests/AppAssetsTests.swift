import NgtermDesign
import Testing
import UIKit

/// The app's assets agree with the design system: the accent color is the tint, the launch screen stands on the
/// grouped ground, and the icon is there.
@MainActor
@Suite("app assets")
internal struct AppAssetsTests {
    @Test("the accent color is the design system's tint", arguments: Appearance.allCases)
    internal func accentIsTheTint(appearance: Appearance) throws {
        let accent = try #require(UIColor(named: "AccentColor"))
        #expect("\(RGB(resolving: accent, in: appearance))" == "\(Palette.tint.resolved(in: appearance))")
    }

    @Test(
        "the launch screen's ground is the grouped background the first screen stands on",
        arguments: Appearance.allCases)
    internal func launchGround(appearance: Appearance) throws {
        let launch = try #require(Bundle.main.object(forInfoDictionaryKey: "UILaunchScreen") as? [String: Any])
        let name = try #require(launch["UIColorName"] as? String)
        let color = try #require(UIColor(named: name))
        #expect(
            "\(RGB(resolving: color, in: appearance))" == "\(RGB(resolving: .systemGroupedBackground, in: appearance))")
    }

    @Test("the app icon is compiled in")
    internal func appIcon() throws {
        let icons = try #require(Bundle.main.object(forInfoDictionaryKey: "CFBundleIcons") as? [String: Any])
        let primary = try #require(icons["CFBundlePrimaryIcon"] as? [String: Any])
        #expect(primary["CFBundleIconName"] as? String == "AppIcon")
    }
}
