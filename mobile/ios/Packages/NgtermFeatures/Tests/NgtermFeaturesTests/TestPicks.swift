import NgtermCore

/// Picks for the tests (mobile/android's `testPick`).
internal enum TestPicks {
    private static func item(_ id: String, status: String?) -> RemotePickItem {
        RemotePickItem(id: id, label: id, subtitle: nil, status: status, statusColor: nil, statusShape: nil)
    }

    /// A pick `K1`, "Switch to": `api` (blocked, with a subtitle), `web` (idle) and `docs` (no status).
    internal static func pick(allowCustom: Bool = false, selection: String? = nil) -> RemotePick {
        RemotePick(
            id: "K1",
            items: [
                RemotePickItem(
                    id: "api",
                    label: "api",
                    subtitle: "~/src/api",
                    status: "blocked",
                    statusColor: nil,
                    statusShape: nil
                ),
                item("web", status: "idle"),
                item("docs", status: nil),
            ],
            prompt: "Switch to",
            query: nil,
            allowCustom: allowCustom,
            selection: selection
        )
    }
}
