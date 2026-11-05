import NgtermPlatform

/// A ``NoticeCenter`` that keeps what is up in memory, and every post and withdrawal in order, for the tests.
@MainActor
internal final class RecordingNotices: NoticeCenter {
    /// What was done, in order: a post as `+id`, a withdrawal as `-id`.
    internal private(set) var log: [String] = []
    /// The notices up, by id.
    internal private(set) var posted: [String: Notice] = [:]
    /// What the user answered about notifying.
    internal var answer = NoticePermission.undecided
    /// How many times the app asked the user.
    internal private(set) var requests = 0

    /// The ids of the notices up, sorted.
    internal var upIds: [String] {
        posted.keys.sorted()
    }

    internal func post(_ notice: Notice) {
        log.append("+" + notice.id)
        posted[notice.id] = notice
    }

    internal func withdraw(_ ids: [String]) {
        for id in ids {
            log.append("-" + id)
            posted[id] = nil
        }
    }

    internal func permission() -> NoticePermission {
        answer
    }

    internal func requestPermission() -> Bool {
        requests += 1
        answer = .allowed
        return true
    }

    deinit {
        // nothing to release
    }
}
