import NgtermCore
import NgtermPlatform

/// The background refresh's work: statuses only.
///
/// For each machine of the persisted attached set (iOS usually runs a refresh in a process started afresh, with
/// nothing attached), it connects, lists the machine's sessions, and notifies each attached session's agent that
/// turned waiting or done against the ``StatusBaseline``, then disconnects. It never attaches, so it never takes the
/// presenter role.
///
/// It runs on a core of its own, made for the run and let go after it: the app's core keeps its connections, or the
/// machines a suspend set aside, as they were for the next resume. A machine whose connect would wait on the user (an
/// unknown or changed host key, a sign-in) is let go at once. Cancelled (the system took its time back), it stops
/// waiting and disconnects what it connected.
@preconcurrency
@MainActor
public final class StatusRefresh {
    private let machines: MachineStore
    private let attached: AttachedStore
    private let baseline: StatusBaseline
    private let alerts: Alerts
    private let makeCore: @MainActor () throws -> any MobileCoreClient

    /// Whether there is anything to refresh: a machine with sessions attached.
    public var hasWork: Bool {
        !wanted.isEmpty
    }

    // each machine with sessions attached, with their ids
    private var wanted: [(machine: StoredMachine, sessions: Set<String>)] {
        let sets = attached.attached
        return machines.machines.compactMap { machine in
            guard let sessions = sets[machine.id], !sessions.isEmpty else {
                return nil
            }
            return (machine, sessions)
        }
    }

    /// The refresh of the sessions `attached` holds, of the machines in `machines`, against `baseline`, posting
    /// through `alerts`, each run on a core `makeCore` makes.
    @preconcurrency
    public init(
        machines: MachineStore,
        attached: AttachedStore,
        baseline: StatusBaseline,
        alerts: Alerts,
        makeCore: @escaping @MainActor () throws -> any MobileCoreClient
    ) {
        self.machines = machines
        self.attached = attached
        self.baseline = baseline
        self.alerts = alerts
        self.makeCore = makeCore
    }

    // a connect that would wait on the user goes: nobody is there to answer
    private static func letGoIfWaiting(_ event: MobileEvent, on core: any MobileCoreClient) {
        switch event {
        case .hostKeyUnknown(let machineId, _), .hostKeyChanged(let machineId, _, _),
            .signInRequested(let machineId, _, _):
            try? core.disconnect(machineId: machineId)

        default:
            break
        }
    }

    /// Runs the refresh once: each machine at the same time, then every one disconnected.
    public func run() async {
        let entries = wanted
        guard !entries.isEmpty, let core = try? makeCore() else {
            return
        }
        let watcher = Task { [events = core.events] in
            for await event in events {
                Self.letGoIfWaiting(event, on: core)
            }
        }
        // each machine at once; a cancel of the run cancels each
        let jobs = entries.map { entry in
            Task {
                await refresh(entry.machine, sessions: entry.sessions, on: core)
            }
        }
        await withTaskCancellationHandler {
            for job in jobs {
                await job.value
            }
        } onCancel: {
            for job in jobs {
                job.cancel()
            }
        }
        watcher.cancel()
        for entry in entries {
            try? core.disconnect(machineId: entry.machine.id)
        }
    }

    private func refresh(_ machine: StoredMachine, sessions: Set<String>, on core: any MobileCoreClient) async {
        do {
            try await core.connect(machine: machine.core)
            let listed = try await core.listSessions(machineId: machine.id)
            for session in listed where sessions.contains(session.id) {
                let key = SessionKey(machineId: machine.id, sessionId: session.id)
                if let kind = baseline.turn(key, to: session.status) {
                    alerts.post(turn: kind, of: session, on: machine)
                }
            }
        } catch {
            // unreachable, refused or cancelled: the next refresh tries again
        }
    }

    deinit {
        // nothing to release: each run's core goes with it
    }
}
