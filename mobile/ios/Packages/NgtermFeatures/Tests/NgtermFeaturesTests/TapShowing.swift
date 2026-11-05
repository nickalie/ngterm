import Observation

/// Whether the window still shows the tapped machine, as ``TapAttachTests`` change it.
@Observable
@MainActor
internal final class TapShowing {
    internal var isShown = true

    deinit {
        // nothing to release
    }
}
