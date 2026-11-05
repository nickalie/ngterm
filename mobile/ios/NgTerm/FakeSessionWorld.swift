#if DEBUG
    import NgtermCore
    import NgtermCoreFakes
    import NgtermPlatform

    /// The fake world's machine with a live session, for the session view's UI tests: `split` at `split.lan`
    /// (connecting at once), whose session `work` has two panes side by side.
    ///
    /// Its stream says, as an attach lands, that it is connected, its layout, an agent working, and that this device
    /// leads both panes; when asked, its origin then asks whether to deploy (``deployAsk``), and says what it does with
    /// the answer as the session's context ("Deploying to production", or "Deploy called off"). Each pane's screen
    /// names it on its first line ("P1 ready"), so a test sees which pane shows.
    internal enum FakeSessionWorld {
        /// The machine's id in the store.
        internal static let machineId = "split"
        /// The session's id.
        internal static let sessionId = "work"
        nonisolated private static let columns = 40
        nonisolated private static let rows = 12
        private static let paneIds = ["P1", "P2"]

        /// The question the origin asks when the launch asks for it.
        internal static let deployAsk = RemoteAsk(
            id: "deploy",
            title: "Deploy api to production?",
            message: "The tests passed on main.",
            buttons: [
                RemoteAskButton(id: "deploy", label: "Deploy", hotkey: nil),
                RemoteAskButton(id: "wait", label: "Not yet", hotkey: nil),
            ],
            defaultId: "deploy",
            destructiveId: nil,
            cancelId: "wait",
            style: "terminal",
            align: "right",
            width: nil,
            pane: nil,
            input: nil,
            choices: nil
        )

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

        private static var askEvent: MobileEvent {
            .ask(machineId: machineId, sessionId: sessionId, ask: deployAsk)
        }

        private static func context(_ text: String) -> MobileEvent {
            .sessionContext(machineId: machineId, sessionId: sessionId, context: text)
        }

        /// Saves the machine in `machines`, signing in with `keyId`, and gives `core` its session, whose origin asks
        /// ``deployAsk`` when `asks`.
        @MainActor
        internal static func seed(_ machines: MachineStore, core: FakeMobileCore, keyId: String, asks: Bool) {
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
            core.attachEvents[sessionId] = attachEvents + (asks ? [askEvent] : [])
            core.answerEvents = [
                FakeMobileCore.answerKey(askId: deployAsk.id, button: "deploy"): [context("Deploying to production")],
                FakeMobileCore.answerKey(askId: deployAsk.id, button: nil): [context("Deploy called off")],
            ]
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
