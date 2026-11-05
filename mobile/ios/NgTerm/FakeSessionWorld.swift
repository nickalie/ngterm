#if DEBUG
    import NgtermCore
    import NgtermCoreFakes
    import NgtermPlatform

    /// The fake world's machine with a live session, for the session view's UI tests: `split` at `split.lan`
    /// (connecting at once), whose session `work` has two panes side by side.
    ///
    /// Its stream says, as an attach lands, that it is connected, its layout, an agent working, and that this device
    /// leads both panes. Each pane's screen names it on its first line ("P1 ready"), so a test sees which pane shows.
    internal enum FakeSessionWorld {
        /// The machine's id in the store.
        internal static let machineId = "split"
        /// The session's id.
        internal static let sessionId = "work"
        nonisolated private static let columns = 40
        nonisolated private static let rows = 12
        private static let paneIds = ["P1", "P2"]

        private static var attachEvents: [MobileEvent] {
            let layout = RemoteLayout(panes: paneIds, primary: paneIds[0], axis: "vertical", shown: true)
            let status = RemoteStatus(
                status: "working",
                blink: false,
                color: nil,
                shape: nil,
                pane: nil,
                changedAt: nil
            )
            var events: [MobileEvent] = [
                .sessionConnection(machineId: machineId, sessionId: sessionId, state: .connected),
                .sessionLayout(machineId: machineId, sessionId: sessionId, layout: layout),
                .sessionStatus(machineId: machineId, sessionId: sessionId, status: status, changed: false),
                .sessionContext(machineId: machineId, sessionId: sessionId, context: "Split the guide"),
            ]
            for paneId in layout.panes {
                let pane = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
                events.append(.paneLead(pane: pane, role: .leader, covered: false))
            }
            return events
        }

        /// Saves the machine in `machines`, signing in with `keyId`, and gives `core` its session.
        @MainActor
        internal static func seed(_ machines: MachineStore, core: FakeMobileCore, keyId: String) {
            let machine = StoredMachine(
                id: machineId,
                label: "split",
                host: "split.lan",
                port: StoredMachine.sshPort,
                user: "nick",
                keyId: keyId
            )
            try? machines.save(machine)
            core.sessions[machineId] = [
                fakeSession(id: sessionId, name: sessionId, paneCount: UInt32(paneIds.count))
            ]
            core.panes[sessionId] = Set(paneIds)
            core.attachEvents[sessionId] = attachEvents
            core.paneFactory = { _, _, paneId, _ in
                FakePane(paneId: paneId) { since in
                    RowBufferWriter(columns: columns, rows: rows, seq: since + 1)
                        .text(0, "\(paneId) ready")
                        .text(1, "$ ")
                        .bytes()
                }
            }
        }
    }
#endif
