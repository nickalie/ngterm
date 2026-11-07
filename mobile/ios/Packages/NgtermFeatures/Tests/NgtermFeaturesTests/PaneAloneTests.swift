import Testing

@testable import NgtermFeatures

/// A pane tab toggling the split (mobile/android's `SessionViewTest` on `PaneAlone`): in the split a tab shows its pane
/// alone, on the pane alone it brings the split back, the other tab shows that pane alone; where the split can't show,
/// it only selects.
@MainActor
@Suite("a pane tab and the split")
internal struct PaneAloneTests {
    private let world = SessionWorld()

    @Test("the split can show only in a regular width, for two panes the origin shows")
    internal func splittable() {
        let split = SessionWorld.split("vertical")
        #expect(PaneAlone.isSplittable(split, isCompact: false))
        #expect(!PaneAlone.isSplittable(split, isCompact: true))
        let hidden = SessionWorld.layout(["P1", "P2"], axis: "vertical", shown: false)
        #expect(!PaneAlone.isSplittable(hidden, isCompact: false))
        #expect(!PaneAlone.isSplittable(SessionWorld.layout(["P1"], axis: nil, shown: true), isCompact: false))
        #expect(!PaneAlone.isSplittable(nil, isCompact: false))
    }

    @Test("a tab in the split shows its pane alone, the shown pane's tab brings the split back, the other's swaps")
    internal func tabs() async throws {
        defer { world.remove() }
        try await world.attach()
        await world.stream(SessionWorld.split("vertical"))
        let model = SessionModel(graph: world.graph, key: SessionWorld.key)
        #expect(model.tapTab("P1", isSplittable: true))
        #expect(model.isAlone)
        #expect(model.focusedPane == "P1")
        #expect(model.tapTab("P2", isSplittable: true))
        #expect(model.isAlone, "the other tab shows that pane alone instead")
        #expect(model.focusedPane == "P2")
        #expect(model.tapTab("P2", isSplittable: true))
        #expect(!model.isAlone, "the shown pane's tab brings the split back")
        // where the split can't show, a tab only selects
        #expect(model.tapTab("P1", isSplittable: false))
        #expect(!model.isAlone)
        #expect(!model.tapTab("P1", isSplittable: false), "the selected pane's tab changes nothing there")
        // the choice is the session's, kept for its next view
        model.tapTab("P1", isSplittable: true)
        #expect(SessionModel(graph: world.graph, key: SessionWorld.key).isAlone)
    }
}
