import NgtermCore
import Testing

/// The upload's stop token: what a cancel tells those waiting on it, before and after it, once or twice.
@Suite("an upload's cancel token", .timeLimit(.minutes(1)))
internal struct UploadCancelTests {
    @Test("a new token is not cancelled; a cancel marks it, and a second one changes nothing")
    internal func cancelOnce() async {
        let cancel = UploadCancel()
        #expect(!cancel.isCancelled)
        cancel.cancel()
        #expect(cancel.isCancelled)
        cancel.cancel()
        #expect(cancel.isCancelled)
        await cancel.cancelled()
    }

    @Test("a wait on a token cancelled already ends at once")
    internal func waitAfterCancel() async {
        let cancel = UploadCancel()
        cancel.cancel()
        await cancel.cancelled()
        await cancel.cancelled()
        #expect(cancel.isCancelled)
    }

    @Test("every wait under way ends at the cancel")
    internal func waitsEndAtCancel() async {
        let cancel = UploadCancel()
        let ended = Counter()
        // immediate tasks run to their first suspension here, so both waits are under way before the cancel
        let waits = (0..<2).map { _ in
            Task.immediate {
                await cancel.cancelled()
                ended.add()
            }
        }
        #expect(ended.count == .zero)
        cancel.cancel()
        for wait in waits {
            await wait.value
        }
        #expect(ended.count == 2)
    }
}
