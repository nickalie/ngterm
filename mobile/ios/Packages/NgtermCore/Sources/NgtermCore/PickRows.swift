import NgtermCoreBindings

/// The rows a pick shows for `query` (crates/ngterm-mobile, picker.rs's `pick_rows`), ranked as desktop's picker ranks
/// them: every item in the origin's order for a blank query, else the items whose label matches, best first; with
/// `allowCustom`, a query matching none offers the custom row alone.
public func pickRows(items: [RemotePickItem], query: String, allowCustom: Bool) -> [PickRow] {
    NgtermCoreBindings.pickRows(items: items, query: query, allowCustom: allowCustom)
}
