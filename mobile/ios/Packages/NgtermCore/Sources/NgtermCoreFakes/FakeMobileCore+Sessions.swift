#if NGTERM_FAKES
    import NgtermCore

    // The fake core's sessions and panes, and the calls it only records.
    extension FakeMobileCore {
        /// Answers the machine's sessions, kept as its listing and delivered as a tree.
        nonisolated(nonsending) public func listSessions(machineId: String) async throws -> [RemoteSession] {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("listSessions", [machineId])
                try await call.pass()
                return try keepListing(machineId)
            }
        }

        /// Adds a session named `name` (a blank name is none), answered once the listing that has it is delivered.
        nonisolated(nonsending) public func createSession(
            machineId: String,
            name: String?
        ) async throws -> RemoteSession {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("createSession", [machineId, name])
                try await call.pass()
                let session = try state.withLock { state throws(MobileError) in
                    _ = try state.sessionsOf(machineId)
                    state.created += 1
                    let named = name.flatMap { $0.allSatisfy(\.isWhitespace) ? nil : $0 }
                    let number = state.created
                    let session = fakeSession(id: "fake-\(number)", name: named ?? "session \(number)")
                    state.sessions[machineId, default: []].append(session)
                    return session
                }
                _ = try keepListing(machineId)
                return session
            }
        }

        /// Ends a session of ``sessions``, delivering the kept listing without it, if one is kept.
        nonisolated(nonsending) public func endSession(machineId: String, sessionId: String) async throws {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("endSession", [machineId, sessionId])
                try await call.pass()
                let (listed, tree) = try state.withLock { state throws(MobileError) in
                    let sessions = try state.sessionsOf(machineId)
                    guard sessions.contains(where: { $0.id == sessionId }) else {
                        throw MobileError.Remote(reason: "no session \(sessionId)")
                    }
                    state.sessions[machineId] = sessions.filter { $0.id != sessionId }
                    guard let listing = state.listings[machineId] else {
                        return (false, [RemoteSession]())
                    }
                    let kept = listing.filter { $0.id != sessionId }
                    state.listings[machineId] = kept
                    return (true, kept)
                }
                if listed {
                    emit(.tree(machineId: machineId, sessions: tree))
                }
            }
        }

        /// Attaches a session of the machine's last listing, then delivers what its stream says (``attachEvents``).
        nonisolated(nonsending) public func attachSession(machineId: String, sessionId: String) async throws {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("attachSession", [machineId, sessionId])
                try await call.pass()
                try state.withLock { state throws(MobileError) in
                    _ = try state.listed(machineId, sessionId)
                    for event in state.attachEvents[sessionId] ?? [] {
                        emit(event, in: &state)
                    }
                }
            }
        }

        /// Hides the session's shown panes.
        public func detachSession(machineId: String, sessionId: String) throws {
            try record("detachSession", [machineId, sessionId])
            state.withLock { state in
                state.shown = state.shown.filter { $0.key.machineId != machineId || $0.key.sessionId != sessionId }
            }
        }

        /// Shows a pane of a session of the machine's last listing: the one shown already, or the factory's.
        nonisolated(nonsending) public func showPane(
            machineId: String,
            sessionId: String,
            paneId: String,
            size: PaneSize
        ) async throws -> any PaneClient {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("showPane", [machineId, sessionId, paneId, size])
                try await call.pass()
                let ref = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
                let (shown, factory) = try state.withLock { state throws(MobileError) in
                    _ = try state.listed(machineId, sessionId)
                    let known = state.listedPanes[machineId]?[sessionId]?.contains(paneId) ?? false
                    guard known || state.panes[sessionId] == nil else {
                        throw MobileError.Remote(reason: Self.unknownPane)
                    }
                    return (state.shown[ref], state.paneFactory)
                }
                if let shown {
                    return shown
                }
                let made = factory(machineId, sessionId, paneId, size)
                return state.withLock { state in
                    if let other = state.shown[ref] {
                        return other
                    }
                    state.shown[ref] = made
                    return made
                }
            }
        }

        /// Hides a shown pane.
        public func hidePane(machineId: String, sessionId: String, paneId: String) throws {
            try record("hidePane", [machineId, sessionId, paneId])
            _ = state.withLock { state in
                state.shown.removeValue(forKey: PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId))
            }
        }

        /// Records the call.
        public func takeLead(machineId: String, sessionId: String, paneId: String) throws {
            try record("takeLead", [machineId, sessionId, paneId])
        }

        /// Records the call.
        public func answerAsk(machineId: String, sessionId: String, askId: String, reply: AskReply) throws {
            try record("answerAsk", [machineId, sessionId, askId, reply])
        }

        /// Records the call.
        public func answerClipboardRead(
            machineId: String,
            sessionId: String,
            paneId: String,
            readId: UInt64,
            reply: ClipboardReply
        ) throws {
            try record("answerClipboardRead", [machineId, sessionId, paneId, readId, reply])
        }

        /// Records the call.
        public func networkChanged() throws {
            try record("networkChanged", [])
        }

        /// Records the call.
        public func suspend() throws {
            try record("suspend", [])
        }

        /// Records the call.
        public func resume() throws {
            try record("resume", [])
        }

        /// Keeps the machine's sessions as its listing and delivers them, as the real core does with a fresh listing.
        private func keepListing(_ machineId: String) throws(MobileError) -> [RemoteSession] {
            let listing = try state.withLock { state throws(MobileError) in try state.keepListing(machineId) }
            emit(.tree(machineId: machineId, sessions: listing))
            return listing
        }
    }
#endif
