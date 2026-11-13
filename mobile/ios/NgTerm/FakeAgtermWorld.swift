#if DEBUG
    import NgtermCore
    import NgtermCoreFakes
    import NgtermPlatform

    /// The fake world's Mac running agterm, for the UI tests of a machine whose app has fewer features: `studio` at
    /// `studio.lan` (connecting at once), whose one session `notes` its listing names as agterm's.
    ///
    /// As an attach lands, its stream says it is connected, its one pane, that Claude Code runs in it and that this
    /// device leads it; the pane's screen says `agterm$`. agterm has no agent chat view, so the pane keeps showing its
    /// terminal, and the fake core refuses its chat calls as the core does.
    internal enum FakeAgtermWorld {
        /// The machine's id in the store.
        internal static let machineId = "studio"
        /// The session's id.
        internal static let sessionId = "notes"
        private static let paneId = "P1"
        nonisolated private static let columns = 40
        nonisolated private static let rows = 12

        private static var attachEvents: [MobileEvent] {
            let layout = RemoteLayout(panes: [paneId], primary: paneId, axis: nil, shown: true)
            let pane = PaneRef(machineId: machineId, sessionId: sessionId, paneId: paneId)
            return [
                .sessionConnection(machineId: machineId, sessionId: sessionId, state: .connected),
                .sessionLayout(machineId: machineId, sessionId: sessionId, layout: layout),
                .paneAgent(
                    machineId: machineId,
                    sessionId: sessionId,
                    paneId: paneId,
                    agent: "claude",
                    conversationId: "a1"
                ),
                .paneLead(pane: pane, role: .leader, covered: false),
            ]
        }

        /// Saves the machine in `machines`, signing in with `keyId`, and gives `core` its session, its other machines'
        /// panes left to the factory it had.
        @MainActor
        internal static func seed(_ machines: MachineStore, core: FakeMobileCore, keyId: String) {
            let machine = StoredMachine(
                id: machineId,
                label: "studio",
                host: "studio.lan",
                port: StoredMachine.sshPort,
                user: "nick",
                keyId: keyId
            )
            try? machines.save(machine)
            core.sessions[machineId] = [fakeSession(id: sessionId, name: sessionId, peer: .agterm)]
            core.panes[sessionId] = [paneId]
            core.attachEvents[sessionId] = attachEvents
            let others = core.paneFactory
            let studio = machineId
            core.paneFactory = { machine, session, pane, size in
                guard machine == studio else {
                    return others(machine, session, pane, size)
                }
                let frames: FakePane.Frames = { since in
                    RowBufferWriter(columns: columns, rows: rows, seq: since + 1)
                        .text(0, "attached agterm-\(session)")
                        .text(1, "agterm$ ")
                        .bytes()
                }
                return FakePane(paneId: pane, frames: frames)
            }
        }
    }
#endif
