#if NGTERM_FAKES
    import NgtermCore

    /// A session as a machine's tree lists it, with what a test doesn't care about left plain; `peer` is the app its
    /// machine runs (`nil`: the listing doesn't say, which reads as ngterm).
    public func fakeSession(
        id: String,
        name: String? = nil,
        parent: String? = nil,
        paneCount: UInt32 = 1,
        peer: PeerKind? = nil
    ) -> RemoteSession {
        RemoteSession(
            id: id,
            name: name ?? id,
            windowName: "main",
            workspaceName: "default",
            context: nil,
            cwd: "/home/fake",
            status: nil,
            statusColor: nil,
            statusShape: nil,
            parent: parent,
            paneCount: paneCount,
            splitAxis: nil,
            peer: peer
        )
    }
#endif
