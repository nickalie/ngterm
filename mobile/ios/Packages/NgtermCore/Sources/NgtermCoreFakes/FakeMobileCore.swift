#if NGTERM_FAKES
    import NgtermCore
    import Synchronization

    /// A ``MobileCoreClient`` with no native library behind it, for the app's tests and its UI tests' Debug builds.
    ///
    /// It mirrors mobile/android's `FakeMobileCore`. It records every call in ``calls``, keeps each machine's sessions
    /// in ``sessions``, hands ``events`` the events a test ``emit(_:)``s (and those it makes itself, all kept in
    /// ``emitted``), and throws the failure a test queued with ``failNext(_:with:)`` from that method's next call. An
    /// async method's next call can be held open with ``hold(_:)``: it is recorded and takes its queued failure at
    /// once, then waits for the gate before going on. A cancel of the caller's task ends its wait at once, as the live
    /// core's does, while the call goes on; a call whose task was cancelled before it is never made, so it is not
    /// recorded and leaves its queued failure and gate to the next one.
    ///
    /// It follows the real core's rules (crates/ngterm-mobile, api.rs and actor.rs) where the app sees them:
    /// - ``connect(machine:)`` reports ``MachineState/connected`` when the machine was not connected (or was connected
    ///   as another record: an edited machine connects anew, one only renamed keeps its connection); a connect that
    ///   fails reports ``MachineState/unreachable(reason:)`` with the failure's reason before it throws
    ///   ``MobileError/Unreachable(reason:)``. ``disconnect(machineId:)`` reports nothing, forgets the machine's
    ///   listing and cancels every connect of it in flight, held or waiting on the user or not
    ///   (``MobileError/Cancelled``); ``useKey(machineId:keyId:)`` puts the key in the record the machine is connected
    ///   with and keeps the connection, so a connect with that key is no edit;
    /// - host keys, when the fake has a host key store and ``hostKeys`` names the key a machine's host offers: a pinned
    ///   key that matches connects; another pinned key reports
    ///   ``MobileEvent/hostKeyChanged(machineId:pinned:offered:)`` and fails (``hostKeyChanged``); no pin reports
    ///   ``MobileEvent/hostKeyUnknown(machineId:fingerprint:)`` and the connect waits for
    ///   ``answerHostKey(machineId:fingerprint:trust:)``, which pins the key and connects when it trusts the key on
    ///   offer, and fails (``hostKeyRefused``) otherwise; a disconnect while it waits cancels it
    ///   (``MobileError/Cancelled``);
    /// - sign-ins, when ``signIns`` names what a machine's host asks the user to sign in with: after its host key, the
    ///   connect reports ``MobileEvent/signInRequested(machineId:message:url:)`` and waits for
    ///   ``finishSignIn(machineId:signedIn:)``, which connects it once signed in and fails it (``signInUnfinished``)
    ///   otherwise; a disconnect while it waits cancels it;
    /// - ``listSessions(machineId:)`` keeps what it answers as the machine's listing and also delivers it as a
    ///   ``MobileEvent/tree(machineId:sessions:)``; ``createSession(machineId:name:)`` does the same with the listing
    ///   that has the new session;
    /// - ``endSession(machineId:sessionId:)`` ends a session of ``sessions`` whether listed or not
    ///   (``MobileError/Remote(reason:)`` for one the machine doesn't have) and delivers a tree only when a listing is
    ///   kept;
    /// - ``attachSession(machineId:sessionId:)`` and ``showPane(machineId:sessionId:paneId:size:)`` take only a session
    ///   of the machine's last listing (``MobileError/NotListed(sessionId:)`` otherwise), not of ``sessions``, as the
    ///   real core refuses a session it has not listed; an attach then delivers the session's ``attachEvents``, as its
    ///   stream would, and an answer to an ask what ``answerEvents`` holds for it;
    /// - a session given ``panes`` has those panes: a show takes only one the session had at the machine's last listing
    ///   (``MobileError/Remote(reason:)`` with ``unknownPane`` otherwise), so a pane added since (a split) needs a new
    ///   listing first. A session not in ``panes`` takes any pane;
    /// - a show hands back the same handle for a pane already shown; a hide, a detach (its session's panes) and a
    ///   disconnect (its machine's) hide panes, and ``shownPanes`` lists those still shown. A new pane is
    ///   ``paneFactory``'s, a ``FakePane`` by default;
    /// - ``openChat(machineId:sessionId:paneId:)`` takes only a shown pane (``MobileError/Remote(reason:)`` with
    ///   ``notShown`` otherwise), delivering the pane's ``chatEvents`` as it opens, and ``openChats`` lists the chats
    ///   open; a close, and hiding the pane, close one.
    ///   ``loadOlder(machineId:sessionId:paneId:)`` answers a chat's next page of ``olderPages`` (empty once none is
    ///   left; ``notOpen`` for a chat not open), and ``agentCommands(machineId:sessionId:paneId:)`` the pane's
    ///   ``catalogs`` entry (``MobileError/Remote(reason:)`` for a pane with none, as an origin without a catalog
    ///   answers). ``uploadFile(machineId:name:bytes:progress:cancel:)`` tells half the bytes sent, waits at the
    ///   test's gate (``MobileError/Cancelled`` once its token is cancelled there), tells all, waits at the
    ///   `uploadFile.saving` gate when a test set one, and answers `/state/uploads/1/<name>`, keeping the upload in
    ///   ``uploads``. Chat events themselves are what a test ``emit(_:)``s.
    public final class FakeMobileCore: MobileCoreClient {
        /// What a show of a pane not shown yet hands back: given the machine, session and pane ids and the size.
        public typealias PaneFactory = @Sendable (String, String, String, PaneSize) -> any PaneClient

        /// The core's reason for a connect refused over a changed host key (transport/russh.rs's `HOST_KEY_CHANGED`).
        public static let hostKeyChanged = "the host key changed"
        /// The core's reason for a pane its session's listing doesn't have (pane.rs's `UNKNOWN_PANE`).
        public static let unknownPane = "the session has no such pane"
        /// The core's reason for a connect whose unknown host key was not trusted (`HOST_KEY_REFUSED`).
        public static let hostKeyRefused = "the host key was not trusted"
        /// The core's reason for a connect whose sign-in was not completed in time (`SIGN_IN_UNFINISHED`).
        public static let signInUnfinished = "the sign-in was not completed in time"
        /// What share of an upload a test's gate holds it at: half of it.
        internal static let halfway: UInt64 = 2
        /// The core's reason for a chat opened on a pane not shown (chat.rs's `NOT_SHOWN`).
        public static let notShown = "the pane is not shown"
        /// The core's reason for a page asked of a chat not open (chat.rs's `NOT_OPEN`).
        public static let notOpen = "the pane's chat is not open"

        /// Every event handed out, those a test emitted and those the fake made, in order.
        public let events: AsyncStream<MobileEvent>
        internal let hosts: (any HostKeyStore)?
        internal let state: Mutex<FakeCoreState>
        private let continuation: AsyncStream<MobileEvent>.Continuation

        /// The calls made so far, oldest first.
        public var calls: [FakeCall] {
            state.withLock { $0.recorded }
        }

        /// The events handed to ``events`` so far, oldest first.
        public var emitted: [MobileEvent] {
            state.withLock { $0.emitted }
        }

        /// The machines connected (and not disconnected since), by id.
        public var connected: Set<String> {
            get { state.withLock { $0.connected } }
            set { state.withLock { $0.connected = newValue } }
        }

        /// Each machine's sessions, by machine id, as ``listSessions(machineId:)`` answers them.
        public var sessions: [String: [RemoteSession]] {
            get { state.withLock { $0.sessions } }
            set { state.withLock { $0.sessions = newValue } }
        }

        /// Each session's panes, by session id, for the sessions a test gives them (see the type's rules).
        public var panes: [String: Set<String>] {
            get { state.withLock { $0.panes } }
            set { state.withLock { $0.panes = newValue } }
        }

        /// The host key each host offers (OpenSSH's `SHA256:` fingerprint), by host name; a host not here offers none.
        public var hostKeys: [String: String] {
            get { state.withLock { $0.hostKeys } }
            set { state.withLock { $0.hostKeys = newValue } }
        }

        /// What each host asks the user to sign in with, by host name.
        public var signIns: [String: FakeSignIn] {
            get { state.withLock { $0.signIns } }
            set { state.withLock { $0.signIns = newValue } }
        }

        /// The machines whose connect waits for ``answerHostKey(machineId:fingerprint:trust:)``, with the key on offer.
        public var waitingForHostKey: [String: String] {
            state.withLock { $0.questions.mapValues(\.offered) }
        }

        /// The machines whose connect waits for ``finishSignIn(machineId:signedIn:)``.
        public var waitingForSignIn: Set<String> {
            state.withLock { Set($0.signInWaits.keys) }
        }

        /// The panes shown and not hidden since.
        public var shownPanes: Set<PaneRef> {
            state.withLock { Set($0.shown.keys) }
        }

        /// What each session's stream says as an attach of it lands, by session id: its state, layout, status and its
        /// panes' leads, as the real core's presentation stream would.
        public var attachEvents: [String: [MobileEvent]] {
            get { state.withLock { $0.attachEvents } }
            set { state.withLock { $0.attachEvents = newValue } }
        }

        /// What the origin tells once an ask is answered, by ``answerKey(askId:button:)``: the stream's word that the
        /// answer arrived (a status, a context), as an origin acting on it would say it.
        public var answerEvents: [String: [MobileEvent]] {
            get { state.withLock { $0.answerEvents } }
            set { state.withLock { $0.answerEvents = newValue } }
        }

        /// What a show of a pane not shown yet hands back.
        public var paneFactory: PaneFactory {
            get { state.withLock { $0.paneFactory } }
            set { state.withLock { $0.paneFactory = newValue } }
        }

        /// The chats open and not closed since.
        public var openChats: Set<PaneRef> {
            state.withLock { $0.chats }
        }

        /// What each pane's chat says as it opens, by pane: its catch-up, as the origin's would.
        public var chatEvents: [PaneRef: [ChatEvent]] {
            get { state.withLock { $0.chatEvents } }
            set { state.withLock { $0.chatEvents = newValue } }
        }

        /// Each pane's older pages, newest first, which a load of older events hands out in turn.
        public var olderPages: [PaneRef: [[ChatEvent]]] {
            get { state.withLock { $0.olderPages } }
            set { state.withLock { $0.olderPages = newValue } }
        }

        /// Each pane's agent commands, which ``agentCommands(machineId:sessionId:paneId:)`` answers.
        public var catalogs: [PaneRef: [AgentCommand]] {
            get { state.withLock { $0.catalogs } }
            set { state.withLock { $0.catalogs = newValue } }
        }

        /// The uploads stored, in order.
        public var uploads: [FakeUpload] {
            state.withLock { $0.uploads }
        }

        /// The cancel token each upload call was given, in order: what the app cancels through.
        public var uploadTokens: [UploadCancel] {
            state.withLock { $0.uploadTokens }
        }

        /// A fake core pinning host keys in `hosts`; with none, it asks about no host key.
        public init(hosts: (any HostKeyStore)? = nil) {
            (events, continuation) = AsyncStream.makeStream(of: MobileEvent.self, bufferingPolicy: .unbounded)
            self.hosts = hosts
            state = Mutex(FakeCoreState { _, _, paneId, _ in FakePane(paneId: paneId) })
        }

        /// The key of ``answerEvents`` for the answer to the ask `askId` with the button `button` (`nil`: escaped).
        public static func answerKey(askId: String, button: String?) -> String {
            "\(askId):\(button ?? "")"
        }

        /// Ends the wait of the connect of `machineId` for the user to sign in: `signedIn` connects it, else it fails.
        public func finishSignIn(machineId: String, signedIn: Bool = true) {
            let wait = state.withLock { $0.signInWaits.removeValue(forKey: machineId) }
            wait?.open(with: signedIn ? .accepted : .refused)
        }

        /// Makes the next call of `method` (its Swift name, `connect` say) throw `error`.
        public func failNext(_ method: String, with error: MobileError) {
            state.withLock { $0.failures[method] = error }
        }

        /// Holds the next call of the async method `method` (its Swift name) open until the gate opens.
        public func hold(_ method: String) -> FakeGate<Void> {
            let gate = FakeGate<Void>()
            state.withLock { $0.holds[method] = gate }
            return gate
        }

        /// Hands `event` to ``events``, as the core's actor would.
        public func emit(_ event: MobileEvent) {
            state.withLock { emit(event, in: &$0) }
        }

        /// Hands `event` to ``events`` under the lock already held on `state`.
        internal func emit(_ event: MobileEvent, in state: inout FakeCoreState) {
            state.emitted.append(event)
            continuation.yield(event)
        }

        /// Records a synchronous call, and throws the failure queued for it.
        internal func record(_ method: String, _ args: [any Hashable & Sendable]) throws(MobileError) {
            let failure = state.withLock { state in
                state.recorded.append(FakeCall(method, args))
                return state.failures.removeValue(forKey: method)
            }
            if let failure {
                throw failure
            }
        }

        /// Records an async call, taking its queued failure and its gate for ``FakeHeldCall/pass()``.
        internal func recordHeld(_ method: String, _ args: [any Hashable & Sendable]) -> FakeHeldCall {
            state.withLock { $0.recordHeld(method, args) }
        }

        /// Reports `machineId` unreachable for `reason`, and throws it.
        internal func unreachable(_ machineId: String, _ reason: String) throws(MobileError) -> Never {
            emit(.machineState(machineId: machineId, state: .unreachable(reason: reason)))
            throw MobileError.Unreachable(reason: reason)
        }

        deinit {
            continuation.finish()
        }
    }
#endif
