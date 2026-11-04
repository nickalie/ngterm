#if DEBUG
    import Foundation
    import NgtermCoreFakes
    import NgtermFeatures
    import NgtermPlatform

    /// The world the app sees on the fake core (the UI tests): stores in a directory of the launch's own, keys and
    /// pins in memory, and hosts that ask what the tests walk through.
    ///
    /// - `build-01.lan` offers ``buildHostKey``, which the first connect asks to trust;
    /// - `signin.lan` offers ``signInHostKey`` and asks to sign in as Tailscale SSH does;
    /// - any other host connects at once.
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

        /// The app's graph over the fake core, with a key already when `withKey`.
        @MainActor
        internal static func graph(withKey: Bool) -> AppGraph {
            let directory = FileManager.default.temporaryDirectory.appending(
                path: "ngterm-fake-\(UUID().uuidString)",
                directoryHint: .isDirectory
            )
            let pins = MemoryHostKeyPins()
            let core = FakeMobileCore(hosts: pins)
            core.hostKeys = ["build-01.lan": buildHostKey, "signin.lan": signInHostKey]
            core.signIns = ["signin.lan": .tailscale]
            let keys = KeyStore(directory: directory)
            if withKey {
                try? keys.add(key)
            }
            return AppGraph(
                machines: MachineStore(directory: directory),
                settings: SettingsStore(directory: directory),
                keys: AppKeys(store: keys, vault: MemoryPrivateKeys(), maker: CoreKeyMaker(), comment: "ngterm@fake"),
                pins: pins,
                core: core
            )
        }
    }
#endif
