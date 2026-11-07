import NgtermCoreBindings

/// The core's `UploadProgress` over the facade's closure.
internal final class ProgressBridge: NgtermCoreBindings.UploadProgress {
    private let told: @Sendable (UInt64, UInt64) -> Void

    internal init(_ told: @escaping @Sendable (UInt64, UInt64) -> Void) {
        self.told = told
    }

    internal func sent(sent: UInt64, total: UInt64) {
        told(sent, total)
    }

    deinit {
        // nothing to release: the closure is the caller's
    }
}
