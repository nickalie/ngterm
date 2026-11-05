import NgtermCore
import NgtermFeatures
import NgtermPlatform
import UIKit

#if DEBUG
    import NgtermCoreFakes
#endif

/// Which core the app runs on, and the graph over it.
///
/// The linked Rust core with the Keychain and Application Support, or, in a Debug build launched with
/// ``CoreChoice/fakeArgument`` (the UI tests), the fake one (``FakeWorld``).
internal struct CoreChoice {
    /// The launch argument that selects the fake core in a Debug build.
    internal static let fakeArgument = "-ngterm-fake-core"
    /// With the fake core, the launch argument that gives the app a key, so the first run is over.
    internal static let fakeKeyArgument = "-ngterm-fake-key"
    /// With the fake core, the launch argument that has two programs ask to read the clipboard at once.
    internal static let fakeClipboardReadArgument = "-ngterm-fake-clipboard-read"
    /// With the fake core, the launch argument that shows a terminal on a fake pane alone (``TerminalBench``), for the
    /// keyboard's UI tests.
    internal static let fakeTerminalArgument = "-ngterm-fake-terminal"
    /// With the fake core, the launch argument that saves a machine with a session of two panes
    /// (``FakeSessionWorld``).
    internal static let fakeSessionArgument = "-ngterm-fake-session"

    // the public key's comment, which shows in a machine's authorized_keys: whose key it is ("ngterm@iPhone")
    @MainActor private static var keyComment: String {
        let device = UIDevice.current.model.filter { $0.isLetter || $0.isNumber || "._-".contains($0) }
        return "ngterm@\(device.isEmpty ? "ios" : device)"
    }

    /// The version of the chosen core.
    internal let version: String
    /// What the app's screens work with.
    internal let graph: AppGraph

    @MainActor
    internal init(arguments: [String]) {
        #if DEBUG
            if arguments.contains(Self.fakeArgument) {
                version = FakeCoreInfo.version
                graph = FakeWorld.graph(
                    withKey: arguments.contains(Self.fakeKeyArgument),
                    readsClipboard: arguments.contains(Self.fakeClipboardReadArgument),
                    hasSession: arguments.contains(Self.fakeSessionArgument)
                )
                return
            }
        #endif
        version = CoreInfo.version
        graph = Self.liveGraph()
    }

    @MainActor
    private static func liveGraph() -> AppGraph {
        let pins = KeychainHostKeys()
        let core: LiveMobileCore
        do {
            core = try LiveMobileCore(vault: KeychainKeyVault(), hosts: pins)
        } catch {
            // the core's actor thread could not start: there is nothing the app can do without it
            fatalError("the mobile core could not start: \(error)")
        }
        return AppGraph(
            machines: MachineStore(),
            settings: SettingsStore(),
            keys: AppKeys(store: KeyStore(), vault: KeychainKeyVault(), maker: CoreKeyMaker(), comment: keyComment),
            pins: pins,
            stores: (attached: AttachedStore(), clipboardTrust: ClipboardTrustStore()),
            core: core
        )
    }
}
