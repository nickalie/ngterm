import NgtermCoreBindings

/// The app's core over the linked Rust one (crates/ngterm-mobile).
///
/// It reaches machines over ssh through russh, with keys from the platform's ``KeyVault`` and host keys pinned in its
/// ``HostKeyStore``, and tells of what happens on ``events``. Dropping it ends ``events``, and the Rust core stops once
/// the calls still in flight have their answers.
public final class LiveMobileCore: MobileCoreClient {
    /// What the core tells the app on its own, fed by its event sink on the core's actor thread.
    public let events: AsyncStream<MobileEvent>
    private let core: MobileCore
    private let sink: CoreEventSink

    /// Starts a core with keys from `vault` and pins in `hosts`, which it calls on its actor thread.
    ///
    /// - Throws: ``MobileError/Stopped`` when the core's actor can't start.
    public init(vault: any KeyVault, hosts: any HostKeyStore) throws {
        let (stream, feed) = CoreEventSink.makeStream()
        core = try MobileCore(sink: feed, vault: VaultBridge(vault), hosts: HostKeysBridge(hosts))
        events = stream
        sink = feed
    }

    /// Connects `machine` (api.rs's `connect`).
    nonisolated(nonsending) public func connect(machine: Machine) async throws {
        try await abandoningOnCancel { [core] in try await core.connect(machine: machine) }
    }

    /// Closes the connection to `machineId` (api.rs's `disconnect`).
    public func disconnect(machineId: String) throws {
        try core.disconnect(machineId: machineId)
    }

    /// Signs `machineId` in with `keyId` from its next attempt (api.rs's `use_key`).
    public func useKey(machineId: String, keyId: String) throws {
        try core.useKey(machineId: machineId, keyId: keyId)
    }

    /// Answers about an unknown host key (api.rs's `answer_host_key`).
    public func answerHostKey(machineId: String, fingerprint: String, trust: Bool) throws {
        try core.answerHostKey(machineId: machineId, fingerprint: fingerprint, trust: trust)
    }

    /// Lists the sessions of `machineId` (api.rs's `list_sessions`).
    nonisolated(nonsending) public func listSessions(machineId: String) async throws -> [RemoteSession] {
        try await abandoningOnCancel { [core] in try await core.listSessions(machineId: machineId) }
    }

    /// Creates a session on `machineId` (api.rs's `create_session`).
    nonisolated(nonsending) public func createSession(machineId: String, name: String?) async throws -> RemoteSession {
        try await abandoningOnCancel { [core] in try await core.createSession(machineId: machineId, name: name) }
    }

    /// Ends a session of `machineId` (api.rs's `end_session`).
    nonisolated(nonsending) public func endSession(machineId: String, sessionId: String) async throws {
        try await abandoningOnCancel { [core] in
            try await core.endSession(machineId: machineId, sessionId: sessionId)
        }
    }

    /// Attaches a session of `machineId` (api.rs's `attach_session`).
    nonisolated(nonsending) public func attachSession(machineId: String, sessionId: String) async throws {
        try await abandoningOnCancel { [core] in
            try await core.attachSession(machineId: machineId, sessionId: sessionId)
        }
    }

    /// Detaches a session of `machineId` (api.rs's `detach_session`).
    public func detachSession(machineId: String, sessionId: String) throws {
        try core.detachSession(machineId: machineId, sessionId: sessionId)
    }

    /// Retries the machines waiting to reconnect (api.rs's `network_changed`).
    public func networkChanged() throws {
        try core.networkChanged()
    }

    /// Closes every connection quietly (api.rs's `suspend`).
    public func suspend() throws {
        try core.suspend()
    }

    /// Brings back what ``suspend()`` closed (api.rs's `resume`).
    public func resume() throws {
        try core.resume()
    }

    /// Answers an ask of an attached session (api.rs's `answer_ask`).
    public func answerAsk(machineId: String, sessionId: String, askId: String, reply: AskReply) throws {
        try core.answerAsk(machineId: machineId, sessionId: sessionId, askId: askId, reply: reply)
    }

    /// Shows a pane of an attached session (api.rs's `show_pane`).
    nonisolated(nonsending) public func showPane(
        machineId: String,
        sessionId: String,
        paneId: String,
        size: PaneSize
    ) async throws -> any PaneClient {
        try await abandoningOnCancel { [core] in
            try await core.showPane(machineId: machineId, sessionId: sessionId, paneId: paneId, size: size)
        }
    }

    /// Hides a shown pane (api.rs's `hide_pane`).
    public func hidePane(machineId: String, sessionId: String, paneId: String) throws {
        try core.hidePane(machineId: machineId, sessionId: sessionId, paneId: paneId)
    }

    /// Takes the lead of a shown pane's session (api.rs's `take_lead`).
    public func takeLead(machineId: String, sessionId: String, paneId: String) throws {
        try core.takeLead(machineId: machineId, sessionId: sessionId, paneId: paneId)
    }

    /// Answers a shown pane's clipboard read (api.rs's `answer_clipboard_read`).
    public func answerClipboardRead(
        machineId: String,
        sessionId: String,
        paneId: String,
        readId: UInt64,
        reply: ClipboardReply
    ) throws {
        try core.answerClipboardRead(
            machineId: machineId,
            sessionId: sessionId,
            paneId: paneId,
            readId: readId,
            reply: reply
        )
    }

    deinit {
        sink.finish()
    }
}
