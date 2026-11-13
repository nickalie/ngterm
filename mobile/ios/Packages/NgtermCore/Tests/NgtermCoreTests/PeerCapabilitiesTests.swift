import NgtermCore
import Testing

/// What each app offers, as the linked core says (peer.rs's `peer_capabilities`).
@Suite("what a machine's app offers")
internal struct PeerCapabilitiesTests {
    @Test("agterm offers no chat view, clipboard bridge, renames or page companions, and says so in one sentence")
    internal func agterm() {
        let offers = peerCapabilities(peer: .agterm)
        #expect(!offers.agentChat && !offers.clipboardBridge && !offers.followsRenames && !offers.pageCompanions)
        #expect(offers.note?.hasPrefix("Attached from agterm") == true)
        #expect(offers.note?.contains("the agent chat view") == true)
    }

    @Test("ngterm offers everything, with nothing to say; a session that doesn't say is ngterm's")
    internal func ngterm() {
        let offers = peerCapabilities(peer: .ngterm)
        #expect(offers.agentChat && offers.clipboardBridge && offers.followsRenames && offers.pageCompanions)
        #expect(offers.note == nil)
        #expect(peerCapabilities(peer: nil) == offers)
    }
}
