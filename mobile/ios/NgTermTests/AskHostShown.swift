import NgtermFeatures
import Observation

/// The session ``AskHost`` shows.
@MainActor
@Observable
internal final class AskHostShown {
    internal var key: SessionKey

    internal init(_ key: SessionKey) {
        self.key = key
    }

    deinit {
        // nothing to release
    }
}
