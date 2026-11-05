#if DEBUG
    import Foundation
    import NgtermCore
    import NgtermCoreFakes
    import NgtermFeatures
    import NgtermPlatform

    /// The world the app sees on the fake core (the UI tests): stores in a directory of the launch's own, keys and
    /// pins in memory, and hosts that ask what the tests walk through.
    ///
    /// - `build-01.lan` offers ``buildHostKey``, which the first connect asks to trust;
    /// - `signin.lan` offers ``signInHostKey`` and asks to sign in as Tailscale SSH does;
    /// - any other host connects at once;
    ///
    /// and, when asked, a program in `docs-site` on `build-01` asks to read the clipboard as the app starts, and one in
    /// `notes` after it: its question shows only once the first is answered; and, when asked, a machine `split`
    /// (`split.lan`) is saved already, with a session `work` of two panes side by side (``FakeSessionWorld``), whose
    /// origin asks a question as it is attached, when asked too.
    ///
    /// Notifications go to memory (``FakeNotices``), and the question about notifying counts as asked unless the
    /// launch asks for it.
    internal enum FakeWorld {
        /// The host key `build-01.lan` offers (made up).
        internal static let buildHostKey = "SHA256:fAkEbU1Ld01h0sTkEyF0rUiTeStS0nLyN0tArEaLkEy00"
        /// The host key `signin.lan` offers (made up).
        internal static let signInHostKey = "SHA256:fAkEs1GnInh0sTkEyF0rUiTeStS0nLyN0tArEaLkEy000"

        // a key as the first run would leave it (made up: the fake core never loads it)
        private static let key = AppKey(
            id: "fake-key",
            authorizedKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForUiTestsOnlyNotARealKey0000000000"
                + " ngterm@fake",
            fingerprint: "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E",
            isImported: false
        )

        /// The app's graph over the fake core, as the launch arguments ask.
        ///
        /// The `arguments` are ``CoreChoice``'s: a key already, two programs asking to read the clipboard, `split` with
        /// its session and its question, the question about notifying left to ask.
        @MainActor
        internal static func graph(arguments: [String]) -> AppGraph {
            let directory = FileManager.default.temporaryDirectory.appending(
                path: "ngterm-fake-\(UUID().uuidString)",
                directoryHint: .isDirectory
            )
            let pins = MemoryHostKeyPins()
            let core = FakeMobileCore(hosts: pins)
            core.hostKeys = ["build-01.lan": buildHostKey, "signin.lan": signInHostKey]
            core.signIns = ["signin.lan": .tailscale]
            let keys = KeyStore(directory: directory)
            if arguments.contains(CoreChoice.fakeKeyArgument) {
                try? keys.add(key)
            }
            let machines = MachineStore(directory: directory)
            if arguments.contains(CoreChoice.fakeSessionArgument) {
                FakeSessionWorld.seed(
                    machines,
                    core: core,
                    keyId: key.id,
                    asks: arguments.contains(CoreChoice.fakeAskArgument)
                )
            }
            let stores = GraphStores(directory: directory)
            if !arguments.contains(CoreChoice.fakeNotificationQuestionArgument) {
                try? stores.notificationPrefs.markPermissionAsked()
            }
            let graph = AppGraph(
                machines: machines,
                settings: SettingsStore(directory: directory),
                keys: AppKeys(store: keys, vault: MemoryPrivateKeys(), maker: CoreKeyMaker(), comment: "ngterm@fake"),
                pins: pins,
                stores: stores,
                core: core,
                notices: FakeNotices()
            )
            if arguments.contains(CoreChoice.fakeClipboardReadArgument) {
                let pane = PaneRef(machineId: "build-01", sessionId: "docs-site", paneId: "P1")
                core.emit(.clipboardReadRequest(pane: pane, readId: 1, clipboard: .standard, listOnly: false))
                // another pane's first read, shown once the first is answered, so a test sees that it was
                let next = PaneRef(machineId: "build-01", sessionId: "notes", paneId: "P2")
                core.emit(.clipboardReadRequest(pane: next, readId: 1, clipboard: .standard, listOnly: false))
            }
            return graph
        }
    }
#endif
