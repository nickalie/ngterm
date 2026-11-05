#if DEBUG
    import NgtermPlatform

    /// The fake world's notifications: kept in memory, so the UI tests raise no system question and post nothing.
    ///
    /// The user hasn't answered about notifying until the app asks, and then allows it.
    @MainActor
    internal final class FakeNotices: NoticeCenter {
        /// The notices up, by id.
        internal private(set) var posted: [String: Notice] = [:]
        private var answer = NoticePermission.undecided

        internal init() {
            // nothing posted yet
        }

        internal func post(_ notice: Notice) {
            posted[notice.id] = notice
        }

        internal func withdraw(_ ids: [String]) {
            for id in ids {
                posted[id] = nil
            }
        }

        internal func permission() -> NoticePermission {
            answer
        }

        internal func requestPermission() -> Bool {
            answer = .allowed
            return true
        }

        deinit {
            // nothing to release
        }
    }
#endif
