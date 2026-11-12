#if NGTERM_FAKES
    import Foundation
    import NgtermCore

    // The fake core's chats: opening and closing them, their older pages, the agents' commands and the uploads.
    extension FakeMobileCore {
        /// Opens the chat of a shown pane, delivering its ``chatEvents``.
        nonisolated(nonsending) public func openChat(
            machineId: String,
            sessionId: String,
            paneId: String
        ) async throws {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("openChat", [machineId, sessionId, paneId])
                try await call.pass()
                let ref = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
                try state.withLock { state throws(MobileError) in
                    _ = try state.sessionsOf(machineId)
                    guard state.shown[ref] != nil else {
                        throw MobileError.Remote(reason: Self.notShown)
                    }
                    state.chats.insert(ref)
                    if let events = state.chatEvents[ref], !events.isEmpty {
                        emit(.chat(pane: ref, events: events), in: &state)
                    }
                }
            }
        }

        /// Closes the pane's chat.
        public func closeChat(machineId: String, sessionId: String, paneId: String) throws {
            try record("closeChat", [machineId, sessionId, paneId])
            _ = state.withLock { state in
                state.chats.remove(PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId))
            }
        }

        /// The open chat's next older page, or none.
        nonisolated(nonsending) public func loadOlder(
            machineId: String,
            sessionId: String,
            paneId: String
        ) async throws -> [ChatEvent] {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("loadOlder", [machineId, sessionId, paneId])
                try await call.pass()
                let ref = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
                return try state.withLock { state throws(MobileError) in
                    guard state.chats.contains(ref) else {
                        throw MobileError.Remote(reason: Self.notOpen)
                    }
                    guard var pages = state.olderPages[ref], !pages.isEmpty else {
                        return []
                    }
                    let page = pages.removeFirst()
                    state.olderPages[ref] = pages
                    return page
                }
            }
        }

        /// The pane's ``catalogs`` entry.
        nonisolated(nonsending) public func agentCommands(
            machineId: String,
            sessionId: String,
            paneId: String
        ) async throws -> [AgentCommand] {
            try await abandoningOnCancel { [self] in
                let call = recordHeld("agentCommands", [machineId, sessionId, paneId])
                try await call.pass()
                let ref = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
                return try state.withLock { state throws(MobileError) in
                    _ = try state.sessionsOf(machineId)
                    guard let commands = state.catalogs[ref] else {
                        throw MobileError.Remote(reason: "no agent in this pane")
                    }
                    return commands
                }
            }
        }

        /// Half the bytes, the test's gate, then all of them, the `uploadFile.saving` gate, and the stored path.
        @preconcurrency
        nonisolated(nonsending) public func uploadFile(
            machineId: String,
            name: String,
            bytes: Data,
            progress: @escaping @Sendable (UInt64, UInt64) -> Void,
            cancel: UploadCancel
        ) async throws -> String {
            try await abandoningOnCancel { [self] in
                let total = UInt64(bytes.count)
                _ = try state.withLock { state throws(MobileError) in
                    _ = try state.sessionsOf(machineId)
                    state.uploadTokens.append(cancel)
                }
                // half of it has gone when a test's gate holds the call; a cancel there stops it
                progress(total / Self.halfway, total)
                let call = recordHeld("uploadFile", [machineId, name, bytes.count])
                try await passUnlessCancelled(call, cancel)
                progress(total, total)
                let saving = state.withLock { $0.holds.removeValue(forKey: "uploadFile.saving") }
                await saving?.wait()
                state.withLock { state in
                    state.uploads.append(FakeUpload(machineId: machineId, name: name, size: bytes.count))
                }
                return "/state/uploads/1/\(name)"
            }
        }

        // the call's gate, unless the upload's token is cancelled first: the bytes still to go never leave
        private func passUnlessCancelled(_ call: FakeHeldCall, _ cancel: UploadCancel) async throws {
            if let hold = call.hold, !cancel.isCancelled {
                let race = FakeRace()
                await race.first(
                    { await cancel.cancelled() },
                    { await hold.wait() }
                )
            }
            if cancel.isCancelled {
                throw MobileError.Cancelled
            }
            try await call.pass()
        }
    }
#endif
