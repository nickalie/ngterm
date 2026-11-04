/// The app's core, as the live one and the fake one both offer it.
///
/// The live one is ``LiveMobileCore``, over crates/ngterm-mobile; the fake one is NgtermCoreFakes' `FakeMobileCore`.
/// Their rules are the Rust core's (api.rs): each call's doc there says what it does and how it fails.
///
/// An async call that has an answer waits for it. A cancel of the caller's task ends that wait at once with
/// `CancellationError`, and the core still carries out what it was asked (as on Android): a cancelled connect goes on
/// connecting until ``disconnect(machineId:)``. A synchronous call hands its command to the core and returns; it throws
/// only ``MobileError/Stopped``.
public protocol MobileCoreClient: AnyObject, Sendable {
    /// What the core tells the app on its own, in the order it told it: one stream for the core's life, for one
    /// consumer, which never waits on the core to take an event. It ends once the core is gone.
    var events: AsyncStream<MobileEvent> { get }

    /// Connects `machine`, answered once it is connected; a lost connection is retried from then on until
    /// ``disconnect(machineId:)``.
    func connect(machine: Machine) async throws

    /// Closes the connection to `machineId` and cancels a connect in flight (``MobileError/Cancelled``).
    func disconnect(machineId: String) throws

    /// `machineId` signs in with the key `keyId` from its next connection attempt on.
    func useKey(machineId: String, keyId: String) throws

    /// The user's answer about the unknown host key `fingerprint` of `machineId`.
    func answerHostKey(machineId: String, fingerprint: String, trust: Bool) throws

    /// `machineId`'s sessions, as its tree lists them; they also arrive as ``MobileEvent/tree(machineId:sessions:)``.
    func listSessions(machineId: String) async throws -> [RemoteSession]

    /// New Session on `machineId`, named `name` when given (a blank name is none).
    func createSession(machineId: String, name: String?) async throws -> RemoteSession

    /// End Session: `machineId` closes its session `sessionId`, with no undo.
    func endSession(machineId: String, sessionId: String) async throws

    /// Attaches `sessionId` of `machineId`: its presentation stream opens and stays up until
    /// ``detachSession(machineId:sessionId:)``.
    func attachSession(machineId: String, sessionId: String) async throws

    /// Detaches `sessionId` of `machineId`: its stream ends and its shown panes are hidden.
    func detachSession(machineId: String, sessionId: String) throws

    /// The device's network changed: every machine waiting to reconnect is retried at once.
    func networkChanged() throws

    /// The app has nothing to keep connected: every connection closes quietly until ``resume()``.
    func suspend() throws

    /// Brings back what ``suspend()`` closed.
    func resume() throws

    /// The user's answer to the ask `askId` of an attached session.
    func answerAsk(machineId: String, sessionId: String, askId: String, reply: AskReply) throws

    /// Shows the pane `paneId` of `sessionId` at `size`; showing a shown pane hands back a handle on the same pane.
    func showPane(machineId: String, sessionId: String, paneId: String, size: PaneSize) async throws -> any PaneClient

    /// Hides a shown pane: its handles go dead (``PaneInputError/Closed``).
    func hidePane(machineId: String, sessionId: String, paneId: String) throws

    /// Take control: a shown pane attaches again claiming the lead.
    func takeLead(machineId: String, sessionId: String, paneId: String) throws

    /// The user's answer to a shown pane's clipboard read `readId`.
    func answerClipboardRead(
        machineId: String,
        sessionId: String,
        paneId: String,
        readId: UInt64,
        reply: ClipboardReply
    ) throws
}
