import Foundation
import GameController

/// Whether a hardware keyboard is attached (`GCKeyboard`): it has every key the extra keys have, so they stand aside
/// while one is.
@preconcurrency
@MainActor
public final class HardwareKeyboard {
    private var observers: [any NSObjectProtocol] = []

    /// Whether a keyboard is attached now.
    public private(set) var isAttached: Bool

    /// Runs as a keyboard comes or goes.
    public var onChange: (@MainActor () -> Void)?

    /// The system's keyboards, followed as they come and go.
    public init() {
        isAttached = GCKeyboard.coalesced != nil
        let center = NotificationCenter.default
        for name in [Notification.Name.GCKeyboardDidConnect, .GCKeyboardDidDisconnect] {
            let observer = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    self?.update(attached: GCKeyboard.coalesced != nil)
                }
            }
            observers.append(observer)
        }
    }

    /// A keyboard that stays attached, or not, whatever the system's do: for tests and the fake core's screens.
    public init(fixed attached: Bool) {
        isAttached = attached
    }

    /// Says a keyboard came (`attached`) or went.
    public func update(attached: Bool) {
        guard attached != isAttached else {
            return
        }
        isAttached = attached
        onChange?()
    }

    isolated deinit {
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
        }
    }
}
