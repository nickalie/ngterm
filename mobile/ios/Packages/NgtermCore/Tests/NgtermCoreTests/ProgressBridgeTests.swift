import Synchronization
import Testing

@testable import NgtermCore

/// The upload progress the core reports, handed to the facade's closure.
@Suite("the upload progress bridge")
internal struct ProgressBridgeTests {
    @Test("each report reaches the closure as sent and total, in order")
    internal func reportsReachTheClosure() {
        let heard = Mutex<[[UInt64]]>([])
        let bridge = ProgressBridge { sent, total in
            heard.withLock { $0.append([sent, total]) }
        }
        bridge.sent(sent: 0, total: 10)
        bridge.sent(sent: 4, total: 10)
        bridge.sent(sent: 10, total: 10)
        #expect(heard.withLock { $0 } == [[0, 10], [4, 10], [10, 10]])
    }
}
