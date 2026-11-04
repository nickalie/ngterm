#if NGTERM_FAKES
    import NgtermCore

    // The fake core's connections: connect and disconnect, keys, host keys and sign-ins.
    extension FakeMobileCore {
        /// Connects `machine`, as the type's rules say.
        nonisolated(nonsending) public func connect(machine: Machine) async throws {
            try await abandoningOnCancel { [self] in
                let (call, attempt) = state.withLock { state in
                    (state.recordHeld("connect", [machine]), state.attempts[machine.id, default: 0])
                }
                do {
                    try await call.pass()
                } catch MobileError.Unreachable(let reason) {
                    try goesOn(machine.id, attempt)
                    try unreachable(machine.id, reason)
                }
                try goesOn(machine.id, attempt)
                guard !keepsConnection(machine) else {
                    return
                }
                state.withLock { $0.drop(machine.id) }
                try await checkHostKey(machine)
                try goesOn(machine.id, attempt)
                try await checkSignIn(machine)
                try state.withLock { state throws(MobileError) in
                    // checked under the lock that connects it: a disconnect comes before it (and cancels) or after
                    guard state.attempts[machine.id, default: 0] == attempt else {
                        throw MobileError.Cancelled
                    }
                    state.connected.insert(machine.id)
                    state.records[machine.id] = machine
                    emit(.machineState(machineId: machine.id, state: .connected), in: &state)
                }
            }
        }

        /// Forgets the connection and listing of `machineId`, hides its panes and cancels every connect of it in
        /// flight.
        public func disconnect(machineId: String) throws {
            try record("disconnect", [machineId])
            let (question, signIn) = state.withLock { state in
                state.attempts[machineId, default: 0] += 1
                state.drop(machineId)
                return (
                    state.questions.removeValue(forKey: machineId),
                    state.signInWaits.removeValue(forKey: machineId)
                )
            }
            question?.answer.open(with: .overtaken)
            signIn?.open(with: .overtaken)
        }

        /// Puts `keyId` in the record `machineId` is connected with, keeping the connection.
        public func useKey(machineId: String, keyId: String) throws {
            try record("useKey", [machineId, keyId])
            state.withLock { $0.records[machineId]?.keyId = keyId }
        }

        /// Answers the question of the connect of `machineId`.
        ///
        /// An answer about another key than the one on offer refuses, since the user never saw that key.
        public func answerHostKey(machineId: String, fingerprint: String, trust: Bool) throws {
            try record("answerHostKey", [machineId, fingerprint, trust])
            let question = state.withLock { $0.questions.removeValue(forKey: machineId) }
            question?.answer.open(with: trust && fingerprint == question?.offered ? .accepted : .refused)
        }

        /// Throws ``MobileError/Cancelled`` once a disconnect of `machineId` overtook its connect made at `attempt`.
        private func goesOn(_ machineId: String, _ attempt: UInt64) throws(MobileError) {
            guard state.withLock({ $0.attempts[machineId, default: 0] }) == attempt else {
                throw MobileError.Cancelled
            }
        }

        /// Whether `machine` is connected as it is, up to its label.
        ///
        /// Renamed only, it keeps its connection and takes the new label.
        private func keepsConnection(_ machine: Machine) -> Bool {
            state.withLock { state in
                guard state.connected.contains(machine.id), var kept = state.records[machine.id] else {
                    return false
                }
                kept.label = machine.label
                guard kept == machine else {
                    return false
                }
                state.records[machine.id] = machine
                return true
            }
        }

        private func checkHostKey(_ machine: Machine) async throws(MobileError) {
            guard let hosts, let offered = state.withLock({ $0.hostKeys[machine.host] }) else {
                return
            }
            let pinned = hosts.pinned(machineId: machine.id)
            guard pinned != offered else {
                return
            }
            if let pinned {
                emit(.hostKeyChanged(machineId: machine.id, pinned: pinned, offered: offered))
                try unreachable(machine.id, Self.hostKeyChanged)
            }
            let answer = FakeGate<FakeCoreState.Outcome>()
            state.withLock { $0.questions[machine.id] = FakeCoreState.Question(offered: offered, answer: answer) }
            emit(.hostKeyUnknown(machineId: machine.id, fingerprint: offered))
            let outcome = await answer.wait()
            state.withLock { state in
                if state.questions[machine.id]?.answer === answer {
                    state.questions[machine.id] = nil
                }
            }
            switch outcome {
            case .accepted:
                hosts.pin(machineId: machine.id, fingerprint: offered)

            case .overtaken:
                throw MobileError.Cancelled

            case .refused:
                try unreachable(machine.id, Self.hostKeyRefused)
            }
        }

        private func checkSignIn(_ machine: Machine) async throws(MobileError) {
            guard let signIn = state.withLock({ $0.signIns[machine.host] }) else {
                return
            }
            let wait = FakeGate<FakeCoreState.Outcome>()
            state.withLock { $0.signInWaits[machine.id] = wait }
            emit(.signInRequested(machineId: machine.id, message: signIn.message, url: signIn.url))
            let outcome = await wait.wait()
            state.withLock { state in
                if state.signInWaits[machine.id] === wait {
                    state.signInWaits[machine.id] = nil
                }
            }
            switch outcome {
            case .accepted:
                return

            case .overtaken:
                throw MobileError.Cancelled

            case .refused:
                try unreachable(machine.id, Self.signInUnfinished)
            }
        }
    }
#endif
