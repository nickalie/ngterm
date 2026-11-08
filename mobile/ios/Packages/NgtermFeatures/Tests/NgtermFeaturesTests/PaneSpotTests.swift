import NgtermCore
import NgtermPlatform
import NgtermTerminal
import Testing

@testable import NgtermFeatures

/// A pane as the session view places it (mobile/android's `PaneSpot`): what covers it in the terminal and in the chat
/// view, and the extra keys' layout Settings asks for.
@MainActor
@Suite("a pane's spot")
internal struct PaneSpotTests {
    private static let ref = PaneRef(machineId: "m1", sessionId: "s1", paneId: "P2")

    private static func spot(live: PaneLive?, failure: String?, isChatView: Bool) -> PaneSpot {
        PaneSpot(ref: ref, shown: nil, live: live, failure: failure, isChatView: isChatView)
    }

    @Test("a pane is named by its id in the session")
    internal func id() {
        #expect(Self.spot(live: nil, failure: nil, isChatView: false).id == "P2")
    }

    @Test("in the terminal, whatever keeps the pane from use covers it")
    internal func terminalCover() {
        #expect(Self.spot(live: nil, failure: nil, isChatView: false).cover == .attaching)
        let lost = PaneLive(role: .follower, isCovered: true, end: .lost(reason: "eof"))
        #expect(Self.spot(live: lost, failure: nil, isChatView: false).cover == .attaching, "no terminal yet")
        #expect(Self.spot(live: nil, failure: "busy", isChatView: false).cover == .failed(reason: "busy"))
    }

    @Test("a chat is covered only by a failed show: reading a chat never takes control")
    internal func chatCover() {
        #expect(Self.spot(live: nil, failure: nil, isChatView: true).cover == nil, "no terminal is needed")
        let follower = PaneLive(role: .follower, isCovered: true)
        #expect(Self.spot(live: follower, failure: nil, isChatView: true).cover == nil)
        let exited = PaneLive(end: .exited(code: 1, signal: nil))
        #expect(Self.spot(live: exited, failure: nil, isChatView: true).cover == nil)
        #expect(Self.spot(live: follower, failure: "busy", isChatView: true).cover == .failed(reason: "busy"))
    }

    @Test("the extra keys are laid out as Settings asks", arguments: ExtraKeysChoice.allCases)
    internal func extraKeys(choice: ExtraKeysChoice) {
        let layouts: [ExtraKeysChoice: ExtraKeysLayout] = [.fit: .fit, .twoLines: .twoLines, .hidden: .hidden]
        #expect(ExtraKeysLayout(choice) == layouts[choice])
    }
}
