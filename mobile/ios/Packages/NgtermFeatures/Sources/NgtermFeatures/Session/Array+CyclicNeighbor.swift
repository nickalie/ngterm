extension Array where Element: Equatable {
    /// The element after (or before) `current`, wrapping around, from the first when `current` isn't here; `nil` with
    /// fewer than two: the next pane of a session, the next attached session.
    internal func cyclicNeighbor(of current: Element?, forward: Bool) -> Element? {
        guard count > 1 else {
            return nil
        }
        let index = current.flatMap(firstIndex(of:)) ?? 0
        let step = forward ? 1 : -1
        return self[(index + step + count) % count]
    }
}
