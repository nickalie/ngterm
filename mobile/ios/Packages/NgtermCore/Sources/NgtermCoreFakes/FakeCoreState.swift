#if NGTERM_FAKES
    import NgtermCore

    /// What a ``FakeMobileCore`` holds, under its lock.
    internal struct FakeCoreState {
        /// How a connect's wait for the user ended.
        internal enum Outcome {
            /// The user accepted: trusted the host key, signed in.
            case accepted
            /// A disconnect overtook the wait.
            case overtaken
            /// The user refused, or didn't finish.
            case refused
        }

        /// A connect's question about an unknown host key: the key on offer, and the gate its answer opens.
        internal struct Question {
            internal let offered: String
            internal let answer: FakeGate<Outcome>
        }

        internal var recorded: [FakeCall] = []
        internal var emitted: [MobileEvent] = []
        internal var failures: [String: MobileError] = [:]
        internal var holds: [String: FakeGate<Void>] = [:]
        // each machine's disconnects so far: a connect made before the latest one is cancelled
        internal var attempts: [String: UInt64] = [:]
        // each machine's record as connected, by id
        internal var records: [String: Machine] = [:]
        internal var connected: Set<String> = []
        internal var hostKeys: [String: String] = [:]
        internal var questions: [String: Question] = [:]
        internal var signIns: [String: FakeSignIn] = [:]
        // each machine's connect waiting for the user to sign in
        internal var signInWaits: [String: FakeGate<Outcome>] = [:]
        internal var created = 0
        internal var sessions: [String: [RemoteSession]] = [:]
        // each machine's last listing: what listSessions or createSession last answered
        internal var listings: [String: [RemoteSession]] = [:]
        internal var panes: [String: Set<String>] = [:]
        // each machine's sessions' panes at its last listing, by machine id then session id
        internal var listedPanes: [String: [String: Set<String>]] = [:]
        internal var shown: [PaneRef: any PaneClient] = [:]
        internal var paneFactory: FakeMobileCore.PaneFactory

        /// Records an async call, taking its queued failure and its gate for ``FakeHeldCall/pass()``.
        internal mutating func recordHeld(_ method: String, _ args: [any Hashable & Sendable]) -> FakeHeldCall {
            recorded.append(FakeCall(method, args))
            return FakeHeldCall(failure: failures.removeValue(forKey: method), hold: holds.removeValue(forKey: method))
        }

        /// The sessions of `machineId`, which must be connected.
        internal func sessionsOf(_ machineId: String) throws(MobileError) -> [RemoteSession] {
            guard connected.contains(machineId) else {
                throw MobileError.NotConnected(machineId: machineId)
            }
            return sessions[machineId] ?? []
        }

        /// The session `sessionId` of the last listing of `machineId`.
        internal func listed(_ machineId: String, _ sessionId: String) throws(MobileError) -> RemoteSession {
            _ = try sessionsOf(machineId)
            guard let session = listings[machineId]?.first(where: { $0.id == sessionId }) else {
                throw MobileError.NotListed(sessionId: sessionId)
            }
            return session
        }

        /// Keeps the sessions of `machineId` as its listing, with their panes, and hands them back.
        internal mutating func keepListing(_ machineId: String) throws(MobileError) -> [RemoteSession] {
            let listing = try sessionsOf(machineId)
            listings[machineId] = listing
            var listed: [String: Set<String>] = [:]
            for session in listing {
                listed[session.id] = panes[session.id]
            }
            listedPanes[machineId] = listed
            return listing
        }

        /// Forgets the connection of `machineId`, its listing and its shown panes.
        internal mutating func drop(_ machineId: String) {
            shown = shown.filter { $0.key.machineId != machineId }
            connected.remove(machineId)
            records[machineId] = nil
            listings[machineId] = nil
            listedPanes[machineId] = nil
        }
    }
#endif
