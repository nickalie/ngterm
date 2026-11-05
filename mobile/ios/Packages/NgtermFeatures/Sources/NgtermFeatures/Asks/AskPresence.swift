/// Whether an ask prompt's view shows, read after the fact: a reference, so a decision made later sees the view's last
/// word even once the view has gone.
@MainActor
internal final class AskPresence {
    /// Whether the view shows, its window not in the background.
    internal var isShown = false

    deinit {
        // nothing to release
    }
}
