import NgtermCore
import UIKit

/// The hardware keys a terminal took while they are held (mobile/android's `HeldChords`, and the keys' own repeat): a
/// key the program got goes on repeating while held and is let go with the modifiers it was pressed with, and a key an
/// app chord took stays the app's until it is let go.
@preconcurrency
@MainActor
internal final class HeldKeys {
    /// What a held key is.
    internal enum Held {
        /// The program's key, pressed as `key`; the system sees it too when `shared` (a modifier key).
        case program(PaneKey, shared: Bool)
        /// An app chord's: nothing of it reaches the program.
        case chord
    }

    private var held: [UIKeyboardHIDUsage: Held] = [:]
    private var repeating: UIKeyboardHIDUsage?

    /// The held key's repeat.
    internal let repeats = KeyRepeat()

    /// The key at `code` went down as `what`.
    internal func hold(_ code: UIKeyboardHIDUsage, as what: Held) {
        held[code] = what
    }

    /// The key at `code` repeats with `action` while held; the one repeating before stops.
    internal func repeatKey(_ code: UIKeyboardHIDUsage, with action: @escaping @MainActor () -> Void) {
        repeating = code
        repeats.start(action)
    }

    /// The key at `code` was let go: what it was held as, if it was.
    internal func letGo(_ code: UIKeyboardHIDUsage) -> Held? {
        if repeating == code {
            repeats.stop()
            repeating = nil
        }
        return held.removeValue(forKey: code)
    }

    /// Lets every key go (the terminal lost the keyboard): the program's keys, to be released.
    internal func letGoAll() -> [PaneKey] {
        repeats.stop()
        repeating = nil
        let keys = held.values.compactMap { what -> PaneKey? in
            if case .program(let key, _) = what {
                return key
            }
            return nil
        }
        held = [:]
        return keys
    }

    deinit {
        // the repeat stops as it goes
    }
}
