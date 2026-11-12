import NgtermCore
import Observation

/// The panes this app shows, and the pane each attached session has selected (mobile/android's `SessionPanes`, with
/// the session view's focus).
///
/// A pane is shown the first time its view has room for it (``show(_:size:)``) and stays shown, keeping its terminal
/// and its lead, while the user is elsewhere in the app, until its session is detached (the core hides it then) or the
/// session's layout drops it (it is hidden). Why a pane could not be shown is in ``failures`` until the next try, or
/// until its machine's link comes back after it was lost, when its view tries again; a show that fails on the lost
/// link after it came back is tried again at once. The core finds a pane through its machine's last listing, so a pane
/// added since (a split) is shown after a fresh listing, and ``retry(_:size:)`` always lists first. Its frames reach
/// its ``ShownPane/feed`` through the live sessions' routing.
///
/// The selection (``focus``) is kept per session, so a trip elsewhere and back, a new window size or another size
/// class finds the same pane; it goes with its session's detach. So is whether a session shows its selected pane
/// alone where the window could show the origin's split (``alone``): the device's own choice, which the origin never
/// hears of, kept while the app runs.
///
/// A session created and opened here claims its lead (``claimLead(of:)``), taken at its first follower report with no
/// Take control cover in between; a detach or a lost link drops the claim, as another device may lead by then.
@preconcurrency
@MainActor
@Observable
public final class SessionPanes {
    /// The core's words for a pane its session's listing doesn't have (crates/ngterm-mobile, pane.rs's `UNKNOWN_PANE`).
    public static let unknownPane = "the session has no such pane"

    /// The shown panes, by pane.
    public private(set) var shown: [PaneRef: ShownPane] = [:]
    /// Why a pane could not be shown (the core's words), by pane, until it is tried again.
    public private(set) var failures: [PaneRef: String] = [:]
    /// The pane each session has selected, by session; one with none selects its layout's primary pane.
    public private(set) var focus: [SessionKey: String] = [:]
    /// The sessions showing their selected pane alone, where the window could show the origin's split.
    public private(set) var alone: Set<SessionKey> = []
    internal private(set) var claims: Set<SessionKey> = []
    // the pane of each claimed session whose takeover is on its way, until its next lead report
    private var taking: [SessionKey: String] = [:]
    @ObservationIgnored private let connections: Connections
    @ObservationIgnored private let live: LiveSessions
    @ObservationIgnored private var showing: Set<PaneRef> = []
    // each machine's link as last seen, and how many times it came back: a show begun before the last return is stale
    @ObservationIgnored private var links: [String: Link] = [:]
    @ObservationIgnored private var returns: [String: Int] = [:]

    /// The panes over `connections`, whose frames go through `live`.
    public init(connections: Connections, live: LiveSessions) {
        self.connections = connections
        self.live = live
        connections.listen { [weak self] event in
            self?.receive(event)
        }
        connections.watchAttached { [weak self] attached in
            self?.attachedChanged(attached)
        }
    }

    // a failure a fresh listing may cure: the session or the pane is missing from the machine's last listing
    private static func isUnlisted(_ error: MobileError) -> Bool {
        switch error {
        case .NotListed:
            true

        case .Remote(let reason):
            reason == unknownPane

        default:
            false
        }
    }

    /// Shows `pane` at `size` unless it is shown or on its way; it lands in ``shown``.
    public func show(_ pane: PaneRef, size: PaneSize) {
        show(pane, size: size, listFirst: false)
    }

    /// Tries `pane` again at `size` (a cover's Retry): its machine is listed afresh, then it is shown as
    /// ``show(_:size:)``.
    public func retry(_ pane: PaneRef, size: PaneSize) {
        show(pane, size: size, listFirst: true)
    }

    /// Hides `pane`: its channel closes, and it goes from ``shown``.
    public func hide(_ pane: PaneRef) {
        // a core that has stopped shows nothing any more
        try? connections.core.hidePane(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
        drop(pane)
    }

    /// Take control of `pane`: it attaches again claiming the lead.
    public func takeLead(_ pane: PaneRef) {
        // a core that has stopped has nothing to lead
        try? connections.core.takeLead(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
    }

    /// Has the first pane of `session` to report a follower take the lead, as the origin leads a session it starts.
    public func claimLead(of session: SessionKey) {
        guard connections.link(of: session.machineId) == .connected else {
            return
        }
        claims.insert(session)
    }

    /// Whether this device is taking the lead of `session`: a Take control cover there would only flash.
    public func isTakingLead(_ session: SessionKey) -> Bool {
        claims.contains(session) || taking[session] != nil
    }

    /// Selects the pane `paneId` of `session`: the one every key goes to.
    public func select(_ paneId: String, of session: SessionKey) {
        focus[session] = paneId
    }

    /// Shows the selected pane of `session` alone (`isAlone`), or the origin's split again, where the window can show
    /// it.
    public func setAlone(_ isAlone: Bool, of session: SessionKey) {
        if isAlone {
            alone.insert(session)
        } else {
            alone.remove(session)
        }
    }

    private func show(_ pane: PaneRef, size: PaneSize, listFirst: Bool) {
        guard shown[pane] == nil, showing.insert(pane).inserted else {
            return
        }
        failures[pane] = nil
        let began = returns[pane.machineId]
        Task {
            var again = false
            do {
                let made = ShownPane(client: try await open(pane, size: size, listFirst: listFirst))
                live.route(pane, to: made.feed)
                shown[pane] = made
                // detached while it was on its way: the core opened it for a session it no longer streams
                if !isAttached(pane) {
                    hide(pane)
                }
            } catch let error as MobileError {
                // a session detached meanwhile keeps no failure to block its next show, and a failure from before its
                // machine came back is stale: the pane is tried again on the link that's back
                if isAttached(pane) {
                    if returns[pane.machineId] == began {
                        failures[pane] = error.reason
                    } else {
                        again = true
                    }
                }
            } catch {
                // the show's task was cancelled: the next view tries again
            }
            showing.remove(pane)
            if again {
                show(pane, size: size, listFirst: listFirst)
            }
        }
    }

    private func open(_ pane: PaneRef, size: PaneSize, listFirst: Bool) async throws -> any PaneClient {
        if listFirst {
            await connections.listSessions(machineId: pane.machineId)
        }
        let core = connections.core
        do {
            return try await core.showPane(
                machineId: pane.machineId,
                sessionId: pane.sessionId,
                paneId: pane.paneId,
                size: size
            )
        } catch let error as MobileError where !listFirst && Self.isUnlisted(error) {
            await connections.listSessions(machineId: pane.machineId)
            return try await core.showPane(
                machineId: pane.machineId,
                sessionId: pane.sessionId,
                paneId: pane.paneId,
                size: size
            )
        }
    }

    private func receive(_ event: MobileEvent) {
        switch event {
        case .machineState(let machineId, _):
            linked(machineId)

        case .sessionLayout(let machineId, let sessionId, let layout):
            // a pane the layout no longer has is hidden, and reports nothing more to release its takeover
            for pane in shown.keys
            where pane.machineId == machineId && pane.sessionId == sessionId && !layout.panes.contains(pane.paneId) {
                hide(pane)
            }
            let session = SessionKey(machineId: machineId, sessionId: sessionId)
            if let pane = taking[session], !layout.panes.contains(pane) {
                taking[session] = nil
            }

        case .paneLead(let pane, let role, _):
            lead(pane, role)

        case .paneEnded(let pane, _):
            release(pane)

        default:
            break
        }
    }

    // an unowned pane reattaches to lead by itself and reports again
    private func lead(_ pane: PaneRef, _ role: PaneRole?) {
        let session = SessionKey(pane)
        release(pane)
        // a late report of a pane the layout dropped leaves the claim to a pane still there
        if let layout = live.sessions[session]?.layout, !layout.panes.contains(pane.paneId) {
            return
        }
        switch role {
        case .follower:
            if claims.remove(session) != nil {
                take(pane)
            }

        case .leader:
            claims.remove(session)

        default:
            break
        }
    }

    private func release(_ pane: PaneRef) {
        let session = SessionKey(pane)
        if taking[session] == pane.paneId {
            taking[session] = nil
        }
    }

    private func take(_ pane: PaneRef) {
        let session = SessionKey(pane)
        taking[session] = pane.paneId
        do {
            try connections.core.takeLead(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
        } catch {
            // no takeover is coming to release the hold
            taking[session] = nil
        }
    }

    // a machine whose link comes back after it was lost lets go of its panes' failures, so their views try again
    private func linked(_ machineId: String) {
        let now = connections.link(of: machineId)
        let before = links.updateValue(now, forKey: machineId)
        if now != .connected {
            claims = claims.filter { $0.machineId != machineId }
            taking = taking.filter { $0.key.machineId != machineId }
        }
        guard now == .connected, before != .connected else {
            return
        }
        returns[machineId, default: 0] += 1
        failures = failures.filter { $0.key.machineId != machineId }
    }

    // a detached session's panes went with it, and why they couldn't be shown, and its selection
    private func attachedChanged(_ attached: [String: Set<String>]) {
        let isKept = { (pane: PaneRef) in attached[pane.machineId]?.contains(pane.sessionId) == true }
        failures = failures.filter { isKept($0.key) }
        for pane in shown.keys where !isKept(pane) {
            drop(pane)
        }
        focus = focus.filter { attached[$0.key.machineId]?.contains($0.key.sessionId) == true }
        claims = claims.filter { attached[$0.machineId]?.contains($0.sessionId) == true }
        taking = taking.filter { attached[$0.key.machineId]?.contains($0.key.sessionId) == true }
    }

    private func isAttached(_ pane: PaneRef) -> Bool {
        connections.attached[pane.machineId]?.contains(pane.sessionId) == true
    }

    private func drop(_ pane: PaneRef) {
        live.route(pane, to: nil)
        shown[pane] = nil
        failures[pane] = nil
    }

    deinit {
        // nothing to release: the connections hold only weak references back
    }
}
