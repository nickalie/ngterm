import NgtermCore
import Testing

/// A pick's rows, as the linked core ranks them (picker.rs's `pick_rows`).
@Suite("a pick's rows")
internal struct PickRowsTests {
    private static let items = ["api", "web", "new tab", "newsletter worker"].map { label in
        RemotePickItem(id: label, label: label, subtitle: "worker", status: nil, statusColor: nil, statusShape: nil)
    }

    private static func rows(_ query: String, allowCustom: Bool = false) -> [PickRow] {
        pickRows(items: items, query: query, allowCustom: allowCustom)
    }

    @Test("a blank query keeps every item in the origin's order")
    internal func blank() {
        #expect(Self.rows(" \n") == (0..<4).map { PickRow.item(index: UInt32($0)) })
    }

    @Test("a query matches the labels only, best first")
    internal func labels() {
        #expect(Self.rows("new") == [.item(index: 2), .item(index: 3)])
        #expect(Self.rows("worker") == [.item(index: 3)])
    }

    @Test("an unmatched query is the answer only when the pick allows one")
    internal func custom() {
        #expect(Self.rows("zzz").isEmpty)
        #expect(Self.rows(" zzz top ", allowCustom: true) == [.custom(label: "Use \"zzz top\"", query: "zzz top")])
    }
}
