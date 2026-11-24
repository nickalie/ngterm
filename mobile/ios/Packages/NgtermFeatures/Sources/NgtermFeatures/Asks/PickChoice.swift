import NgtermCore

/// What a pick's rows answer (mobile/android's `initialRow` and `replyFor`).
internal enum PickChoice {
    /// The row Return answers with at open: the item `selection` names when `rows` shows it, else the first.
    internal static func initialRow(_ rows: [PickRow], items: [RemotePickItem], selection: String?) -> Int {
        guard let selection else {
            return 0
        }
        let found = rows.firstIndex { row in
            guard case .item(let index) = row, Int(index) < items.count else {
                return false
            }
            return items[Int(index)].id == selection
        }
        return found ?? 0
    }

    /// What answering with `row` sends: the item's id, or the custom text.
    internal static func reply(_ row: PickRow, items: [RemotePickItem]) -> PickReply {
        switch row {
        case .custom(_, let query):
            return .custom(query: query)

        case .item(let index):
            guard Int(index) < items.count else {
                return .cancelled
            }
            return .picked(item: items[Int(index)].id)
        }
    }
}
