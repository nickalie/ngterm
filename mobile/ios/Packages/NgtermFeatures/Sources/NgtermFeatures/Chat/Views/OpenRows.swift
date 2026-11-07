/// The open rows' keys, with the conversation they were opened in: a row's key (a transcript line's offset) may come
/// again in another conversation, so they open nothing there.
internal struct OpenRows: Equatable {
    internal var conversation: String?
    internal var keys: Set<String> = []

    /// The keys open in `current`.
    internal func of(_ current: String?) -> Set<String> {
        conversation == current ? keys : []
    }

    /// The rows of `current` with `key` opened, or closed when it is open.
    internal func toggled(_ current: String?, _ key: String) -> Self {
        var now = of(current)
        if now.contains(key) {
            now.remove(key)
        } else {
            now.insert(key)
        }
        return Self(conversation: current, keys: now)
    }
}
