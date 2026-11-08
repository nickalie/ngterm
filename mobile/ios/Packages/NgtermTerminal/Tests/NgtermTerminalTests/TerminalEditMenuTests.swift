import Testing
import UIKit

@testable import NgtermTerminal

/// The edit menu as the system's menu interaction asks it: what it points at, and its going.
@MainActor
@Suite("the edit menu")
internal struct TerminalEditMenuTests {
    /// The system's animator for a menu going: it keeps what it is given, as the system's would until it runs.
    private final class Animator: NSObject, UIEditMenuInteractionAnimating {
        private var kept: [() -> Void] = []

        func addAnimations(_ animations: @escaping () -> Void) {
            kept.append(animations)
        }

        func addCompletion(_ completion: @escaping () -> Void) {
            kept.append(completion)
        }

        deinit {
            // nothing to release
        }
    }

    // what the system hands the menu's delegate
    private static var interaction: UIEditMenuInteraction {
        UIEditMenuInteraction(delegate: nil)
    }

    private static var configuration: UIEditMenuConfiguration {
        UIEditMenuConfiguration(identifier: nil, sourcePoint: .zero)
    }

    @Test("over a touch selection it points at the selection's rows; at a secondary click, at no rows")
    internal func target() throws {
        let rig = TerminalRig(clipboard: nil)
        let menu = rig.view.editMenu
        rig.view.gestures.longPress(.began, at: rig.point(2, 1))
        rig.show([SelectedSpan(row: 1, first: 0, last: 5)])
        rig.view.gestures.longPress(.ended, at: rig.point(2, 1))
        let spots = try #require(rig.view.handleSpots)
        #expect(rig.view.isMenuShown)
        #expect(menu.editMenuInteraction(Self.interaction, targetRectFor: Self.configuration) == spots.rows)
        rig.view.pointerGestures.press(.began, at: rig.point(9, 3), buttons: .secondary, mods: [], time: 1)
        #expect(menu.editMenuInteraction(Self.interaction, targetRectFor: Self.configuration).isNull)
    }

    @Test("the system taking the menu down leaves it gone, and a selection's next frame doesn't bring it back")
    internal func dismissed() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.pointerGestures.press(.began, at: rig.point(4, 2), buttons: .secondary, mods: [], time: 1)
        #expect(rig.view.isMenuShown)
        rig.view.editMenu.editMenuInteraction(
            Self.interaction,
            willDismissMenuFor: Self.configuration,
            animator: Animator()
        )
        #expect(!rig.view.isMenuShown)
        rig.view.drawFrame()
        #expect(!rig.view.isMenuShown)
    }
}
