import Foundation
import NgtermCore
import NgtermPlatform
import Observation

/// Settings over the graph's stores (mobile/android's `SettingsViewModel`): the app's settings, the notification
/// switches, the machines trusted with the clipboard, and the keys.
///
/// The stores are read at launch, so there is nothing to load: what they hold shows as it changes. A change a store
/// can't write leaves it as it was and says why in ``failure``, but for a clipboard Ask again
/// (``revokeClipboard(_:)``), which still takes effect until the app closes. A key made (``rotate()``) or brought
/// (``importKey(privateKey:passphrase:)``) lands in ``newKey``, for its public half to be shown; ``useEverywhere(_:)``
/// then points every machine at it.
@preconcurrency
@MainActor
@Observable
public final class SettingsModel {
    /// Where making or bringing a key stands.
    public private(set) var work = KeyWork()
    /// The id of a key just made or brought, until ``newKeyShown()``.
    public private(set) var newKey: String?
    /// Why the last change couldn't be kept, until ``dismissFailure()``.
    public private(set) var failure: String?
    /// Whether the app may notify, as last read (``readPermission()``); `nil` until then.
    public private(set) var permission: NoticePermission?
    @ObservationIgnored private let graph: AppGraph

    /// The app's settings.
    public var settings: AppSettings {
        graph.settings.settings
    }

    /// The notification kinds that are on.
    public var enabledNotices: Set<NoticeKind> {
        graph.notificationPrefs.prefs.enabled
    }

    /// The app's keys, oldest first, with the machines that use each.
    public var keys: [KeyItem] {
        let machines = graph.machines.machines
        return graph.keys.keys.map { KeyItem(key: $0, machines: machines) }
    }

    /// The machines whose programs read the clipboard unasked, in the machines' order; one removed since isn't listed.
    public var trusted: [TrustedMachine] {
        let allowed = graph.clipboardReads.alwaysAllowed
        return graph.machines.machines.filter { allowed.contains($0.id) }.map(TrustedMachine.init)
    }

    /// Settings over `graph`.
    public init(graph: AppGraph) {
        self.graph = graph
    }

    /// Changes the app's settings by `edit`.
    public func change(_ edit: (inout AppSettings) -> Void) {
        do {
            try graph.settings.update(edit)
            failure = nil
        } catch {
            failure = String(localized: "The setting couldn't be saved: \(error.description)")
        }
    }

    /// Turns the notifications of `kind` on or off.
    public func setNotice(_ kind: NoticeKind, isOn: Bool) {
        do {
            try graph.notificationPrefs.set(kind, isOn: isOn)
            failure = nil
        } catch {
            failure = String(localized: "The notification setting couldn't be saved: \(error.description)")
        }
    }

    /// The programs of the machine `machineId` ask again before they read the clipboard.
    ///
    /// A revoke the store can't keep still asks again until the app ends, and ``failure`` says so.
    public func revokeClipboard(_ machineId: String) {
        do {
            try graph.clipboardReads.revoke(machineId)
            failure = nil
        } catch {
            failure = String(
                localized: "It asks again until ngterm closes, but that couldn't be saved: \(error.description)"
            )
        }
    }

    /// Reads again whether the app may notify: the user may have changed it in the system's settings.
    public func readPermission() async {
        permission = await graph.notices.permission()
    }

    /// Asks the user whether the app may notify, if they haven't answered yet; the question about notifying is then
    /// asked, so it doesn't come again as a session is attached.
    public func askPermission() async {
        guard await graph.notices.permission() == .undecided else {
            await readPermission()
            return
        }
        // a choice the store can't keep asks again at the next attach, which is no harm
        try? graph.notificationPrefs.markPermissionAsked()
        _ = await graph.notices.requestPermission()
        await readPermission()
    }

    /// Makes a new key, which the machines don't use until ``useEverywhere(_:)``.
    ///
    /// Nothing happens while one is being made.
    public func rotate() async {
        guard !work.isRotating else {
            return
        }
        work.isRotating = true
        work.rotateFailure = nil
        do {
            newKey = try await graph.keys.rotate().id
        } catch {
            work.rotateFailure = ErrorWords.of(error)
        }
        work.isRotating = false
    }

    /// Opens the import afresh: no problem from an earlier attempt; an import still under way goes on.
    public func startImport() {
        work.importProblem = nil
    }

    /// The chosen file could not be read.
    public func unreadable() {
        work.importProblem = .unreadable
    }

    /// Imports the key in `privateKey` (zeroed once read), opened with `passphrase`; while one runs, a second is
    /// dropped (its bytes zeroed).
    public func importKey(privateKey: inout Data, passphrase: String) async {
        guard !work.isImporting else {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
            return
        }
        guard privateKey.contains(where: { !Character(UnicodeScalar($0)).isWhitespace }) else {
            privateKey.resetBytes(in: privateKey.startIndex..<privateKey.endIndex)
            work.importProblem = .empty
            return
        }
        work.isImporting = true
        work.importProblem = nil
        do {
            newKey = try await graph.keys.importKey(privateKey: &privateKey, passphrase: passphrase).id
        } catch let error as KeyError {
            work.importProblem = ImportProblem(error)
        } catch {
            work.importProblem = .notStored(reason: ErrorWords.of(error))
        }
        work.isImporting = false
    }

    /// The new key's public half is on screen.
    public func newKeyShown() {
        newKey = nil
    }

    /// Points every machine at the key `keyId`, from their next connection: the stored machines, and the records the
    /// core holds for the ones it connects (a reconnect signs in with it too), their connections staying.
    public func useEverywhere(_ keyId: String) {
        do {
            try graph.machines.useKey(keyId)
        } catch {
            failure = String(localized: "The machines couldn't be switched to the key: \(error.description)")
            return
        }
        failure = nil
        for machine in graph.machines.machines {
            graph.connections.useKey(machineId: machine.id, keyId: keyId)
        }
    }

    /// The failure has been read.
    public func dismissFailure() {
        failure = nil
    }

    deinit {
        // nothing to release: the stores are the graph's
    }
}
