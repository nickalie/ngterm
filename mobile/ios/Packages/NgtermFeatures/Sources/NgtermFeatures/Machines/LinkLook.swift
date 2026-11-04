import Foundation
import NgtermDesign

/// How a machine's ``Link`` shows on its row: the badge's tone and words, what the state means, and the one action it
/// calls for.
internal struct LinkLook: Equatable {
    /// What a row's action does.
    internal enum Action: Equatable {
        /// Connect, Retry, Retry now.
        case connect(String)
        /// Review a changed host key.
        case review
    }

    internal let tone: StatusTone
    internal let label: String?
    internal let detail: String?
    internal let action: Action?

    /// The look of `link` at `now` (a reconnect counts down to its retry).
    internal init(_ link: Link, now: Date) {
        switch link {
        case .offline:
            self.init(
                .offline,
                detail: String(localized: "Not connected"),
                action: .connect(String(localized: "Connect"))
            )

        case .connecting:
            self.init(.connecting)

        case .connected:
            self.init(.online)

        case .reconnecting(let retryAt):
            let seconds = Int(retryAt.timeIntervalSince(now).rounded(.up))
            self.init(
                .connecting,
                label: seconds > 0
                    ? String(localized: "Reconnecting in \(seconds) s") : String(localized: "Reconnecting"),
                detail: String(localized: "The connection dropped; trying again"),
                action: .connect(String(localized: "Retry now"))
            )

        case .unreachable(let reason):
            self.init(
                .failed,
                label: String(localized: "Unreachable"),
                detail: reason,
                action: .connect(String(localized: "Retry"))
            )

        case .hostKeyUnknown:
            // the question itself is the sheet over the app, open while the connect waits
            self.init(.attention, detail: String(localized: "Confirm its host key to connect"))

        case .signIn:
            // the link and its Open link are the sheet over the app, open while the connect waits
            self.init(
                .attention,
                label: String(localized: "Waiting for sign-in"),
                detail: String(localized: "Sign in at its link to connect")
            )

        case .hostKeyChanged:
            self.init(
                .failed,
                label: String(localized: "Host key changed"),
                detail: String(localized: "Not connecting until you review it"),
                action: .review
            )
        }
    }

    private init(_ tone: StatusTone, label: String? = nil, detail: String? = nil, action: Action? = nil) {
        self.tone = tone
        self.label = label
        self.detail = detail
        self.action = action
    }
}
