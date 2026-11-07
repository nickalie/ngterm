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
            // a pane the layout no longer has is hidden
            for pane in shown.keys
            where pane.machineId == machineId && pane.sessionId == sessionId && !layout.panes.contains(pane.paneId) {
                hide(pane)
            }

        default:
            break
        }
    }

    // a machine whose link comes back after it was lost lets go of its panes' failures, so their views try again
    private func linked(_ machineId: String) {
        let now = connections.link(of: machineId)
        let before = links.updateValue(now, forKey: machineId)
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
