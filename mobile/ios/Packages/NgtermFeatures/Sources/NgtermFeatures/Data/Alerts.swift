import NgtermCore
import NgtermPlatform

/// What reaches the user as notifications while the core is connected (mobile/android's `Alerts`).
///
/// It posts, from the core's events and the open ``Asks``:
/// - a session's agent's notifications (``NoticeKind/notices``), taken down when the session is detached;
/// - an agent status change to waiting (`blocked`) or done (`completed`) while its session isn't on screen, replaced by
///   the next change and taken down by one to anything else, by the session coming on screen or being detached;
/// - a machine with attached sessions lost for good (``NoticeKind/errors``: unreachable, or its host key changed),
///   taken down when it connects again;
/// - every open ask out of sight (the ask rule: a sheet while its session is on screen, a notification otherwise),
///   taken down when it is answered, dismissed, taken back, or comes on screen.
///
/// The kinds the user turned off are not posted. Names come from the machines and their listings (an attached session's
/// under the name its stream last said), and a session's context from its stream once it has said one, its listing's
/// until then. Each notice groups under its session's thread (a machine's own under
/// the machine's). A tap opens its session and pane (``OpenRequests``); an ask's action answers it through the core,
/// in the grace too (``respond(to:)``).
@preconcurrency
@MainActor
public final class Alerts {
    private let connections: Connections
    private let asks: Asks
    private let live: LiveSessions
    private let machines: MachineStore
    private let prefs: NotificationPrefsStore
    private let center: any NoticeCenter
    private let requests: OpenRequests
    // the machines whose host key changed since they were last connected
    private var keyChanged: Set<String> = []
    // the asks' notices up, by id
    private var posted: Set<String> = []
    // the agents' notices up, by session: taken down with its detach, since their tap leads to an attached session
    private var notices: [SessionKey: Set<String>] = [:]
    private var lastNotice: UInt64 = 0
    // the attached set as last seen
    private var attached: Set<SessionKey> = []

    /// The alerts over `connections` and `asks`, taking contexts from `live`, naming machines from `machines`, posting
    /// the kinds `prefs` has on to `center`, and handing a tap's target to `requests`.
    @preconcurrency
    public init(
        connections: Connections,
        asks: Asks,
        live: LiveSessions,
        machines: MachineStore,
        prefs: NotificationPrefsStore,
        center: any NoticeCenter,
        requests: OpenRequests
    ) {
        self.connections = connections
        self.asks = asks
        self.live = live
        self.machines = machines
        self.prefs = prefs
        self.center = center
        self.requests = requests
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.attachedChanged(attached)
        }
        asks.watch { [weak self] in
            self?.syncAsks()
        }
    }

    /// Posts that the agent of `session`, an attached session of `machine`, turned `kind` (a background refresh's
    /// finding), unless the user turned that kind off.
    public func post(turn kind: NoticeKind, of session: RemoteSession, on machine: StoredMachine) {
        guard prefs.prefs.enabled.contains(kind) else {
            return
        }
        let key = SessionKey(machineId: machine.id, sessionId: session.id)
        let subject = AlertSubject(key: key, machine: machine.title, session: session.name)
        center.post(Alert.status(about: subject, kind: kind, context: session.context, pane: nil).notice)
    }

    /// Waits until every notice posted or taken down so far has reached the system (a background refresh's last step:
    /// the system may suspend the process once its task ends).
    public func delivered() async {
        await center.delivered()
    }

    /// What the user did with a notice: a tap opens its target; an ask's action answers the ask through the core, and
    /// an ask no longer open (answered or taken back meanwhile) has its notice taken down, which nothing else would.
    public func respond(to response: NoticeResponse) {
        let info = response.info
        guard let machineId = info[Alert.machineInfo] else {
            return
        }
        guard let action = response.action else {
            requests.request(
                OpenTarget(machineId: machineId, sessionId: info[Alert.sessionInfo], paneId: info[Alert.paneInfo])
            )
            return
        }
        guard action.hasPrefix(Alert.answerPrefix), let sessionId = info[Alert.sessionInfo],
            let askId = info[Alert.askInfo]
        else {
            return
        }
        let key = SessionKey(machineId: machineId, sessionId: sessionId)
        let button = String(action.dropFirst(Alert.answerPrefix.count))
        guard let open = asks.find(key, askId: askId) else {
            takeDown([response.noticeId])
            return
        }
        let reply = AskDraft.notificationReply(open.ask, button: button, typed: response.text)
        asks.answer(key, askId: askId, reply: reply)
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .notify(let machineId, let sessionId, let notice):
            self.notice(SessionKey(machineId: machineId, sessionId: sessionId), notice)

        case .sessionStatus(let machineId, let sessionId, let status, let changed):
            if changed {
                self.status(SessionKey(machineId: machineId, sessionId: sessionId), status)
            }

        case .machineState(let machineId, .unreachable(let reason)):
            // a changed host key is the reason the core gives up: its own alert says more
            if !keyChanged.contains(machineId) {
                lost(machineId, .unreachable(reason))
            }

        case .machineState(let machineId, .connected):
            keyChanged.remove(machineId)
            takeDown([Alert.lostId(machineId)])

        case .hostKeyChanged(let machineId, _, _):
            keyChanged.insert(machineId)
            lost(machineId, .hostKeyChanged)

        default:
            break
        }
    }

    private func notice(_ key: SessionKey, _ notice: RemoteNotice) {
        guard prefs.prefs.enabled.contains(.notices) else {
            return
        }
        lastNotice += 1
        let alert = Alert.notice(number: lastNotice, about: subject(key), notice: notice)
        notices[key, default: []].insert(alert.id)
        center.post(alert.notice)
    }

    private func status(_ key: SessionKey, _ status: RemoteStatus?) {
        let kind = StatusBaseline.kind(of: status?.status)
        guard let kind, prefs.prefs.enabled.contains(kind), !asks.onScreen.contains(key) else {
            takeDown([Alert.statusId(key)])
            return
        }
        let stream = live.sessions[key]
        let context = stream?.isContextSaid == true ? stream?.context : listing(key)?.context
        center.post(Alert.status(about: subject(key), kind: kind, context: context, pane: status?.pane).notice)
    }

    // only a machine the user has sessions attached on: one without is lost quietly, as its own screen shows
    private func lost(_ machineId: String, _ reason: LostReason) {
        guard prefs.prefs.enabled.contains(.errors), connections.attached[machineId]?.isEmpty == false else {
            return
        }
        center.post(Alert.lost(machineId: machineId, machine: machineName(machineId), reason: reason).notice)
    }

    private func attachedChanged(_ now: [String: Set<String>]) {
        let keys = Set(now.flatMap { machineId, ids in ids.map { SessionKey(machineId: machineId, sessionId: $0) } })
        let gone = attached.subtracting(keys)
        attached = keys
        var ids: [String] = []
        for key in gone {
            ids.append(Alert.statusId(key))
            ids.append(contentsOf: notices.removeValue(forKey: key) ?? [])
        }
        takeDown(ids.sorted())
    }

    private func syncAsks() {
        let shown = asks.onScreen
        takeDown(shown.map(Alert.statusId).sorted())
        var wanted: [String: OpenAsk] = [:]
        for open in asks.open where !shown.contains(open.key) {
            wanted[Alert.askId(open.key, askId: open.ask.id)] = open
        }
        takeDown(posted.subtracting(wanted.keys).sorted())
        posted.formIntersection(wanted.keys)
        for open in asks.open {
            let id = Alert.askId(open.key, askId: open.ask.id)
            guard wanted[id] != nil, posted.insert(id).inserted else {
                continue
            }
            center.post(Alert.ask(open, about: subject(open.key), delivery: AskDelivery(open.ask)).notice)
        }
    }

    private func takeDown(_ ids: [String]) {
        if !ids.isEmpty {
            center.withdraw(ids)
        }
    }

    private func machineName(_ machineId: String) -> String {
        machines.machine(id: machineId)?.title ?? machineId
    }

    private func listing(_ key: SessionKey) -> RemoteSession? {
        connections.trees[key.machineId]?.first { $0.id == key.sessionId }
    }

    private func subject(_ key: SessionKey) -> AlertSubject {
        AlertSubject(key: key, machine: machineName(key.machineId), session: listing(key)?.name ?? key.sessionId)
    }

    deinit {
        // nothing to release: the connections and asks hold only weak references back
    }
}
