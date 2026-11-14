#if DEBUG
    import Dispatch
    import notify
    import Synchronization

    /// What a UI test tells the fake worlds it has seen, so a scripted step that would end a state the test still has
    /// to look at waits for the test instead of a clock: the test posts the cue's Darwin notification
    /// (`FakeApp.cue`), and ``next(_:)`` returns once one has come that no earlier call took.
    ///
    /// The app listens from launch, so a cue posted before its step asks for it waits banked.
    internal final class FakeCues: Sendable {
        /// The cues, by the Darwin notification's name.
        internal enum Cue: String, CaseIterable, Sendable {
            /// The chat's link drops.
            case chatDrop = "com.nickalie.ngterm.fake-cue.chat-drop"
            /// The chat's link comes back.
            case chatBack = "com.nickalie.ngterm.fake-cue.chat-back"
        }

        private struct Line {
            var banked = 0
            var waiting: [CheckedContinuation<Void, Never>] = []
        }

        private let lines = Mutex<[Cue: Line]>([:])

        /// Listens for every cue, for as long as the app runs.
        internal init() {
            for cue in Cue.allCases {
                var token: Int32 = 0
                notify_register_dispatch(cue.rawValue, &token, DispatchQueue.global()) { [weak self] _ in
                    self?.came(cue)
                }
            }
        }

        /// Returns once `cue` has come, each post taken by one call.
        internal func next(_ cue: Cue) async {
            await withCheckedContinuation { waiter in
                let banked = lines.withLock { lines in
                    if lines[cue, default: Line()].banked > 0 {
                        lines[cue, default: Line()].banked -= 1
                        return true
                    }
                    lines[cue, default: Line()].waiting.append(waiter)
                    return false
                }
                if banked {
                    waiter.resume()
                }
            }
        }

        /// Whether a post of `cue` waits for a step to take it.
        internal func isBanked(_ cue: Cue) -> Bool {
            lines.withLock { ($0[cue]?.banked ?? 0) > 0 }
        }

        private func came(_ cue: Cue) {
            let waiter = lines.withLock { lines -> CheckedContinuation<Void, Never>? in
                guard lines[cue, default: Line()].waiting.isEmpty else {
                    return lines[cue, default: Line()].waiting.removeFirst()
                }
                lines[cue, default: Line()].banked += 1
                return nil
            }
            waiter?.resume()
        }

        deinit {
            // nothing to release: the app's lifetime bounds the registrations
        }
    }

#endif
