#if NGTERM_FAKES
    import NgtermCore

    /// An async call of a ``FakeMobileCore``, recorded as it was made: the failure queued for it and the gate holding
    /// it, both taken at once, so a later call goes on unheld.
    internal struct FakeHeldCall: Sendable {
        internal let failure: MobileError?
        internal let hold: FakeGate<Void>?

        /// Waits at the call's gate, if any, then throws its queued failure, if any.
        internal func pass() async throws(MobileError) {
            await hold?.wait()
            if let failure {
                throw failure
            }
        }
    }
#endif
