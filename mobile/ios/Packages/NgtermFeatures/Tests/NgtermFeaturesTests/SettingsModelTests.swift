import Foundation
import NgtermCore
import NgtermPlatform
import Observation
import Synchronization
import Testing

@testable import NgtermFeatures

/// Settings over the graph's stores, as mobile/android's `SettingsViewModelTest` has them: the settings, the
/// notification switches and the permission, the machines trusted with the clipboard, and a store that can't write.
@MainActor
@Suite("settings")
internal struct SettingsModelTests {
    private let test = TestGraph()

    // a program on `machineId` reads the clipboard and the user allows it always
    private func trust(_ machineId: String) async {
        let pane = PaneRef(machineId: machineId, sessionId: "s1", paneId: "P1")
        test.core.emit(
            .clipboardReadRequest(
                pane: pane,
                readId: 1,
                clipboard: .standard,
                listOnly: false,
                mimes: ["text/plain"],
                list: false,
                granted: false,
                canRemember: false
            )
        )
        await test.settled()
        let reads = test.graph.clipboardReads
        guard let read = reads.pending.first(where: { $0.pane == pane }) else {
            Issue.record("no read from \(machineId)")
            return
        }
        reads.allowAlways(read)
    }

    @Test("a changed setting is stored and shown")
    internal func change() {
        defer { test.remove() }
        let model = SettingsModel(graph: test.graph)
        #expect(model.settings == AppSettings())
        model.change { settings in
            settings.textSize = 16
            settings.palette = .ghostty
            settings.extraKeys = .twoLines
            settings.theme = .dark
        }
        #expect(model.settings.textSize == 16)
        #expect(model.settings.palette == .ghostty)
        let stored = SettingsStore(directory: test.directory).settings
        #expect(stored == AppSettings(textSize: 16, palette: .ghostty, theme: .dark, extraKeys: .twoLines))
        #expect(model.failure == nil)
    }

    @Test("a notification kind turns off and on again, kept across launches")
    internal func notices() {
        defer { test.remove() }
        let model = SettingsModel(graph: test.graph)
        #expect(model.enabledNotices == Set(NoticeKind.allCases))
        model.setNotice(.done, isOn: false)
        #expect(!model.enabledNotices.contains(.done))
        #expect(!NotificationPrefsStore(directory: test.directory).prefs.enabled.contains(.done))
        model.setNotice(.done, isOn: true)
        #expect(model.enabledNotices == Set(NoticeKind.allCases))
    }

    @Test("Turn on asks the system's question once, and only while it hasn't been answered")
    internal func permission() async {
        defer { test.remove() }
        let model = SettingsModel(graph: test.graph)
        #expect(model.permission == nil)
        await model.readPermission()
        #expect(model.permission == .undecided)
        await model.askPermission()
        #expect(test.notices.requests == 1)
        #expect(model.permission == .allowed)
        // the question about notifying isn't asked again as a session is attached
        #expect(test.graph.notificationPrefs.prefs.isPermissionAsked)
        test.notices.answer = .denied
        await model.askPermission()
        #expect(test.notices.requests == 1)
        #expect(model.permission == .denied)
    }

    @Test("a machine trusted with the clipboard is listed until it asks again; one removed since isn't listed")
    internal func clipboard() async throws {
        defer { test.remove() }
        try test.seed(TestGraph.machine(keyId: "k"))
        try test.seed(TestGraph.machine(id: "m2", label: "nas", host: "nas.lan", keyId: "k"))
        let model = SettingsModel(graph: test.graph)
        #expect(model.trusted.isEmpty)
        await trust("m2")
        await trust("gone")
        #expect(model.trusted.map(\.id) == ["m2"])
        #expect(model.trusted.first?.address == "nick@nas.lan:22")
        // the list follows a revoke as it happens
        let changed = Mutex(false)
        withObservationTracking {
            _ = model.trusted
        } onChange: {
            changed.withLock { $0 = true }
        }
        model.revokeClipboard("m2")
        #expect(changed.withLock { $0 })
        #expect(model.trusted.isEmpty)
        #expect(ClipboardTrustStore(directory: test.directory).machines == ["gone"])
    }

    @Test("a change the store can't write leaves the setting and says why until dismissed")
    internal func unwritable() throws {
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: test.directory.path)
            test.remove()
        }
        let model = SettingsModel(graph: test.graph)
        try FileManager.default.createDirectory(at: test.directory, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: test.directory.path)
        model.change { $0.textSize = 20 }
        #expect(model.settings.textSize == AppSettings.defaultTextSize)
        #expect(model.failure?.hasPrefix("The setting couldn't be saved") == true)
        model.setNotice(.waiting, isOn: false)
        #expect(model.enabledNotices.contains(.waiting))
        model.dismissFailure()
        #expect(model.failure == nil)
    }

    @Test("a key's users: every machine with none at all, and none of them for a key no machine signs in with")
    internal func keyItems() {
        let key = AppKey(id: "k", authorizedKey: "ssh-ed25519 AAAA", fingerprint: "SHA256:x", isImported: false)
        let none = KeyItem(key: key, machines: [])
        #expect(none.usedBy.isEmpty)
        #expect(none.isEveryMachine)
        let machines = [
            TestGraph.machine(keyId: "k"),
            TestGraph.machine(id: "m2", label: "nas", host: "nas.lan", keyId: "other"),
        ]
        let some = KeyItem(key: key, machines: machines)
        #expect(some.usedBy == ["build-01"])
        #expect(!some.isEveryMachine)
    }

    @Test("the preview names the palette it draws: the theme's is Copper when dark, Drafting when light")
    internal func paletteNames() {
        #expect(SettingsChoices.name(of: .theme, isDark: true) == "Copper")
        #expect(SettingsChoices.name(of: .theme, isDark: false) == "Drafting")
        #expect(SettingsChoices.name(of: .ghostty, isDark: true) == "Ghostty")
        #expect(SettingsChoices.name(of: .drafting, isDark: true) == "Drafting")
    }

    @Test("a control's binding shows the setting and stores each change as it is made")
    internal func bindings() {
        defer { test.remove() }
        let model = SettingsModel(graph: test.graph)
        let size = model.binding(\.textSize)
        #expect(size.wrappedValue == AppSettings().textSize)
        size.wrappedValue = 18
        #expect(model.settings.textSize == 18)
        #expect(SettingsStore(directory: test.directory).settings.textSize == 18)
        let done = model.noticeBinding(.done)
        #expect(done.wrappedValue)
        done.wrappedValue = false
        #expect(!done.wrappedValue)
        #expect(!NotificationPrefsStore(directory: test.directory).prefs.enabled.contains(.done))
    }
}
