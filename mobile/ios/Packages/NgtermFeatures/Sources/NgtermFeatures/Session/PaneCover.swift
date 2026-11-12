/// Why a pane can't be used as it is, and so what covers it (mobile/android's `PaneCover`).
public enum PaneCover: Equatable, Sendable {
    /// No screen yet: showing, or attaching again.
    case attaching
    /// The program exited with `code` or by `signal`: the session ended there.
    case exited(code: UInt32?, signal: String?)
    /// The pane could not be shown, for `reason` (the core's words).
    case failed(reason: String)
    /// Another client leads: its grid and keys are the other device's.
    case follower
    /// The link dropped; the core attaches it again on its own. The last screen stays readable.
    case reconnecting

    /// Whether it veils the pane from VoiceOver: all but ``reconnecting``, whose last screen stays readable.
    public var veils: Bool {
        self != .reconnecting
    }

    /// What covers a pane: its show's `failure`, no terminal yet (`hasTerminal` false), its attach's end, or its
    /// role's cover (attaching while this device is taking its lead, `isTaking`); `nil` when it is the user's to use.
    public static func of(_ live: PaneLive?, hasTerminal: Bool, failure: String?, isTaking: Bool = false) -> Self? {
        if let failure {
            return .failed(reason: failure)
        }
        guard hasTerminal else {
            return .attaching
        }
        switch live?.end {
        case .lost:
            return .reconnecting

        case .exited(let code, let signal):
            return .exited(code: code, signal: signal)

        case .gone, nil:
            break
        }
        guard live?.isCovered == true else {
            return nil
        }
        return live?.role == .follower && !isTaking ? .follower : .attaching
    }
}
