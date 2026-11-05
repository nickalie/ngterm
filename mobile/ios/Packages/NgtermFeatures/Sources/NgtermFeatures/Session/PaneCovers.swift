import NgtermDesign

/// The veils over a pane that can't be used as it is (mobile/android's `LeadCover`).
///
/// Each has the action that fixes it: **Take control** for a follower, Detach for a program that exited, Retry for a
/// pane that could not be shown. Reconnecting is no veil: it is the ``ReconnectingStrip``.
internal enum PaneCovers {
    /// The veil for `cover` over a pane on `machine`; `nil` for reconnecting, which veils nothing.
    @MainActor
    internal static func veil(
        _ cover: PaneCover,
        machine: String,
        onTakeLead: @escaping @MainActor () -> Void,
        onDetach: @escaping @MainActor () -> Void,
        onRetry: @escaping @MainActor () -> Void
    ) -> Cover? {
        switch cover {
        case .follower:
            Cover(
                systemImage: "hand.raised",
                title: String(localized: "Another device is driving this pane"),
                message: String(
                    localized: """
                        Its size and keys are the other device's. Take control to type here; the other device \
                        follows.
                        """
                ),
                action: RowAction(String(localized: "Take control"), perform: onTakeLead)
            )

        case .attaching:
            Cover(
                systemImage: nil,
                title: String(localized: "Attaching"),
                message: String(localized: "Reading this pane from \(machine).")
            )

        case .reconnecting:
            nil

        case .exited(let code, let signal):
            Cover(
                systemImage: "stop.circle",
                title: String(localized: "The program exited"),
                message: exited(code: code, signal: signal),
                action: RowAction(String(localized: "Detach"), perform: onDetach)
            )

        case .failed(let reason):
            Cover(
                systemImage: "exclamationmark.triangle",
                title: String(localized: "Can't show this pane"),
                message: reason,
                action: RowAction(String(localized: "Retry"), perform: onRetry)
            )
        }
    }

    private static func exited(code: UInt32?, signal: String?) -> String {
        if let signal {
            return String(localized: "Stopped by \(signal). The last screen stays as it was.")
        }
        if let code {
            return String(localized: "Exit code \(Int(code)). The last screen stays as it was.")
        }
        return String(localized: "The last screen stays as it was.")
    }
}
