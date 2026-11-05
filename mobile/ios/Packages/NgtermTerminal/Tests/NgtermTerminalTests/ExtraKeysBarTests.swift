import NgtermTerminal
import SnapshotTesting
import Testing
import UIKit

/// The extra keys' lines by the width they get, how they look, and a composition at the cursor.
///
/// The keys at rest, with Ctrl held for the next key and Alt locked, in light and dark. Recorded at a display scale of
/// 2, one set per simulator.
@MainActor
@Suite("the extra keys")
internal struct ExtraKeysBarTests {
    private static let phone: CGFloat = 390
    private static let wide: CGFloat = 820
    private static let scale: CGFloat = 2

    // the bar of a keyboard with Ctrl held for the next key and Alt locked, `width` points wide in `style`
    private static func bar(width: CGFloat, style: UIUserInterfaceStyle) -> UIView {
        let rig = TerminalRig(clipboard: nil)
        let keyboard = rig.view.keyboard
        keyboard.target = rig.view
        keyboard.toggle(.control)
        keyboard.toggle(.alt)
        keyboard.toggle(.alt)
        let bar = keyboard.bar
        let lines = CGFloat(ExtraKeysBar.rows(forWidth: width).count)
        // the ground the keyboard's backdrop gives it on a device, which a render outside a window leaves out
        let host = UIView(frame: CGRect(x: 0, y: 0, width: width, height: ExtraKeysBar.keyHeight * lines))
        host.backgroundColor = .secondarySystemBackground
        host.traitOverrides.userInterfaceStyle = style
        host.traitOverrides.displayScale = scale
        // the default text size, whatever another test left the process at
        host.traitOverrides.preferredContentSizeCategory = .large
        bar.frame = host.bounds
        host.addSubview(bar)
        host.updateTraitsIfNeeded()
        bar.updateTraitsIfNeeded()
        host.layoutIfNeeded()
        bar.layoutIfNeeded()
        return host
    }

    private static func image(of view: UIView) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        return UIGraphicsImageRenderer(bounds: view.bounds, format: format).image { context in
            view.layer.render(in: context.cgContext)
        }
    }

    private func snapshot(_ view: UIView, named name: String, testName: String = #function, line: UInt = #line) {
        assertSnapshot(
            of: Self.image(of: view),
            as: .image(precision: 1, perceptualPrecision: 0.98),
            named: "\(name)-on-\(SnapshotDevice.current)",
            testName: testName,
            line: line
        )
    }

    @Test("two lines of eight on a phone held upright, one line of sixteen once each key gets 44 points")
    internal func rows() {
        #expect(ExtraKeysBar.rows(forWidth: Self.phone) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: 703) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: 704) == [ExtraKey.oneRow])
        #expect(Set(ExtraKey.twoRows.joined()) == Set(ExtraKey.allCases))
        #expect(Set(ExtraKey.oneRow) == Set(ExtraKey.allCases))
        #expect(ExtraKey.twoRows.allSatisfy { $0.count == 8 })
    }

    @Test("Settings' layouts: two lines at every width, or no bar, with the focus staying")
    internal func layouts() {
        #expect(ExtraKeysBar.rows(forWidth: Self.wide, layout: .twoLines) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: Self.wide, layout: .fit) == [ExtraKey.oneRow])
        let rig = TerminalRig(clipboard: nil)
        let keyboard = rig.view.keyboard
        keyboard.target = rig.view
        #expect(keyboard.showsExtraKeys)
        #expect(rig.view.inputAccessoryView === keyboard.bar)
        let bar = keyboard.bar
        bar.frame = CGRect(x: 0, y: 0, width: Self.wide, height: ExtraKeysBar.keyHeight * 2)
        bar.layoutIfNeeded()
        #expect(bar.rows == [ExtraKey.oneRow])
        keyboard.layout = .twoLines
        bar.layoutIfNeeded()
        #expect(bar.rows == ExtraKey.twoRows)
        keyboard.layout = .hidden
        #expect(!keyboard.showsExtraKeys)
        #expect(rig.view.inputAccessoryView == nil)
        keyboard.layout = .fit
        #expect(rig.view.inputAccessoryView === keyboard.bar)
    }

    @Test("two lines on a phone, light")
    internal func phoneLight() {
        snapshot(Self.bar(width: Self.phone, style: .light), named: "phone-light")
    }

    @Test("two lines on a phone, dark")
    internal func phoneDark() {
        snapshot(Self.bar(width: Self.phone, style: .dark), named: "phone-dark")
    }

    @Test("one line on a wide screen")
    internal func wide() {
        snapshot(Self.bar(width: Self.wide, style: .light), named: "wide")
    }

    @Test("a composition waits at the cursor, in the terminal's face on its ground, underlined in the cursor's color")
    internal func composition() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.setMarkedText("にほん", selectedRange: NSRange(location: 3, length: 0))
        rig.view.displayNow()
        snapshot(rig.view, named: "composition")
    }
}
