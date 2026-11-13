import Foundation

/// What the body of a machine's sessions screen is, by its link and its listing (mobile/android's `SessionsBody`).
public enum SessionsBody: Hashable, Sendable {
    /// Connecting, or waiting for the answer a sheet over the app asks for (a host key, a sign-in).
    case connecting
    /// The machine has no sessions; `failure` is why the last refresh failed, if it did.
    case empty(failure: String?)
    /// The machine's host key changed: nothing connects until it is reviewed.
    case hostKeyChanged
    /// The sessions could not be listed, for `reason`, and there is no earlier listing.
    case listFailed(reason: String)
    /// The machine has no `ngtermctl` on the PATH of ssh logins (the shell's `reason`).
    case missingCli(reason: String)
    /// Not connected and not trying; `isDeviceOffline` when the device has no network either.
    case notConnected(isDeviceOffline: Bool)
    /// Connected; the first listing is on its way.
    case reading
    /// The connection dropped; the core tries again at `retryAt`.
    case reconnecting(retryAt: Date)
    /// The sessions by family (a parent, then its companions right under it), in the machine's order of parents: the
    /// families with a session this app has `attached`, then the `others`. `failure` is why the last refresh failed,
    /// if it did (the list is the one before).
    case sessions(attached: [SessionLine], others: [SessionLine], failure: String?)
    /// The machine can't be reached, for `reason`.
    case unreachable(reason: String)

    // the CLI every remote call runs (ngterm-core's REMOTE_CLI)
    private static let remoteCli = "ngtermctl"

    /// The body for `state`.
    public init(_ state: MachineSessions) {
        switch state.link {
        case .connected:
            self = Self.listing(state)

        case .offline:
            self = .notConnected(isDeviceOffline: state.isOffline)

        // a host key question and a sign-in are sheets over the app while the connect waits
        case .connecting, .hostKeyUnknown, .signIn:
            self = .connecting

        case .reconnecting(let retryAt):
            self = .reconnecting(retryAt: retryAt)

        case .unreachable(let reason):
            self = .unreachable(reason: reason)

        case .hostKeyChanged:
            self = .hostKeyChanged
        }
    }

    /// Whether `reason` is the far shell saying it has no `ngtermctl` (`/bin/sh: 1: ngtermctl: not found`, bash's
    /// `command not found`, env's `No such file or directory`): the core passes the shell's own words on.
    public static func isMissingCli(_ reason: String) -> Bool {
        let text = reason.lowercased()
        return text.contains(remoteCli) && (text.contains("not found") || text.contains("no such file"))
    }

    private static func listing(_ state: MachineSessions) -> Self {
        guard state.isListed else {
            guard let failure = state.failure else {
                return .reading
            }
            return isMissingCli(failure) ? .missingCli(reason: failure) : .listFailed(reason: failure)
        }
        let listed = state.sessions
        if listed.isEmpty {
            return .empty(failure: state.failure)
        }
        let names = Dictionary(listed.map { ($0.id, $0.name) }) { first, _ in first }
        let families = SessionFamilies.of(listed).map { family in
            family.map { session in
                SessionLine(
                    session: session,
                    companionOf: session.parent.map { names[$0] ?? $0 },
                    isAttached: state.attached.contains(session.id),
                    isEnding: state.ending.contains(session.id),
                    isDuplicating: state.duplicating == session.id,
                    canDuplicate: !state.isCreating && !state.ending.contains(session.id)
                )
            }
        }
        let mine = families.filter { family in family.contains(where: \.isAttached) }
        let others = families.filter { family in !family.contains(where: \.isAttached) }
        return .sessions(attached: mine.flatMap(\.self), others: others.flatMap(\.self), failure: state.failure)
    }
}
