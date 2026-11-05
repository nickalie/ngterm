import Foundation
import UIKit

/// Keeps the screen from dimming and locking while any session view on screen asks for it (Android's `KeepScreenOn`,
/// across the iPad's windows): the system's idle timer is off while one does, and on again once none does.
@preconcurrency
@MainActor
public final class ScreenAwake {
    private let apply: @MainActor (Bool) -> Void
    private var holders: Set<UUID> = []

    /// Whether the screen is kept awake now.
    public var isAwake: Bool {
        !holders.isEmpty
    }

    /// Keeps the screen awake through `apply` (the system's idle timer unless a test gives its own).
    @preconcurrency
    public init(apply: @escaping @MainActor (Bool) -> Void = { UIApplication.shared.isIdleTimerDisabled = $0 }) {
        self.apply = apply
    }

    /// The session view `holder` shows a session, and asks for the screen to stay awake when `wanted` (Settings' Keep
    /// screen on); `false` lets its hold go.
    public func hold(_ holder: UUID, wanted: Bool) {
        let before = isAwake
        if wanted {
            holders.insert(holder)
        } else {
            holders.remove(holder)
        }
        if isAwake != before {
            apply(isAwake)
        }
    }

    deinit {
        // the idle timer is the system's, as the last hold left it
    }
}
