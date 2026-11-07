import Foundation
import Observation

/// The sessions whose More menu set their own Agent chat view, against Settings' (mobile/android's
/// `SessionChatViews`): whether each one's panes running an agent show as a chat.
///
/// They are kept as one JSON file in Application Support (`session-chat-views.json`), the newest ``maxSessions``,
/// oldest first, so sessions ended long ago don't pile up; one dropped that way follows Settings again. A choice equal
/// to Settings' is dropped. It reads the file as ``ClipboardTrustStore`` reads its own: one that no longer decodes
/// reads as none (kept as `session-chat-views.json.bad`), and one that can't be read reads as none and is left alone
/// until ``readIfUnread()`` or the next change can read it. A change is written whole before ``views`` shows it.
@preconcurrency
@MainActor
@Observable
public final class SessionChatViewsStore {
    /// One session's own choice, as the file keeps it.
    public struct Entry: Codable, Hashable, Sendable {
        /// The machine's id.
        public let machine: String
        /// The session's id on the machine.
        public let session: String
        /// Whether its panes running an agent show as a chat.
        public let chat: Bool

        /// The choice `chat` of `session` on `machine`.
        public init(machine: String, session: String, chat: Bool) {
            self.machine = machine
            self.session = session
            self.chat = chat
        }
    }

    /// The store's file name.
    public static let fileName = "session-chat-views.json"
    /// The most sessions kept with a choice of their own.
    public static let maxSessions = 256

    /// The sessions' own choices, oldest first, as last written.
    public private(set) var views: [Entry]
    @ObservationIgnored private let file: JSONFile<[Entry]>
    // whether `views` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            views = try file.read() ?? []
            loaded = true
        } catch {
            views = []
            loaded = false
        }
    }

    /// The choice of `session` on `machine`, if it set its own.
    public func choice(machine: String, session: String) -> Entry? {
        views.last { $0.machine == machine && $0.session == session }
    }

    /// Reads the file again if it couldn't be read before; a store that has read it keeps what it has.
    public func readIfUnread() {
        guard !loaded else {
            return
        }
        do {
            views = try file.read() ?? []
            loaded = true
        } catch {
            // still unreadable: no session has a choice of its own, as before
        }
    }

    /// Shows the agents of `session` on `machine` as a chat (`chat`) or as their terminal; a choice equal to
    /// `default`, Settings' own, is dropped, so the session follows Settings again.
    ///
    /// - Throws: ``StoreError/unwritable(reason:)`` when it can't write the change, or
    ///   ``StoreError/unreadable(reason:)`` when the file is there but still can't be read; the choices stay as they
    ///   were.
    public func set(machine: String, session: String, chat: Bool, default: Bool) throws(StoreError) {
        if !loaded {
            views = try file.read() ?? []
            loaded = true
        }
        var next = views.filter { $0.machine != machine || $0.session != session }
        if chat != `default` {
            next.append(Entry(machine: machine, session: session, chat: chat))
        }
        next = Array(next.suffix(Self.maxSessions))
        try file.write(next)
        views = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
