#if DEBUG
    import NgtermCore
    import NgtermCoreFakes
    import NgtermTerminal

    /// What ``TerminalBench`` shows: a fake pane that records the input, a keyboard of its own with no hardware
    /// keyboard (so its extra keys show on a simulator that has the Mac's), and the moves its chords asked for.
    @MainActor
    internal final class TerminalBenchModel {
        nonisolated private static let columns = 40
        nonisolated private static let rows = 12
        private static let modifierKeys: Set = [
            "shift_left", "shift_right", "control_left", "control_right", "alt_left", "alt_right", "meta_left",
            "meta_right", "caps_lock",
        ]

        /// The pane, which records the input.
        internal let pane: FakePane
        /// Where its frame events go.
        internal let feed = TerminalFeed()
        /// The keyboard.
        internal let keyboard = TerminalKeyboard(hardware: HardwareKeyboard(fixed: false))
        /// The moves the chords ran, in order.
        internal private(set) var moves: [String] = []

        /// Everything the pane was typed, run together.
        internal var typed: String {
            pane.inputs.compactMap { input in
                if case .typed(let text) = input {
                    return text
                }
                return nil
            }
            .joined()
        }

        /// The keys the pane was pressed, each as its name and its modifiers' bits (`key_c/2`), the modifier keys left
        /// out.
        internal var keys: String {
            pane.inputs.compactMap { input in
                if case .key(let key) = input, key.action == .press, !Self.modifierKeys.contains(key.key) {
                    return "\(key.key)/\(key.mods)"
                }
                return nil
            }
            .joined(separator: " ")
        }

        /// The grid the pane was given last, which the text size changes.
        internal var grid: String {
            pane.sizes.last.map { "\($0.cols)x\($0.rows)" } ?? ""
        }

        internal init() {
            pane = FakePane(paneId: "bench") { since in
                RowBufferWriter(columns: Self.columns, rows: Self.rows, seq: since + 1).text(0, "$ ").bytes()
            }
            keyboard.shortcuts = TerminalShortcuts(
                nextPane: { [weak self] in self?.moves.append("next pane") },
                previousPane: { [weak self] in self?.moves.append("previous pane") },
                nextSession: { [weak self] in self?.moves.append("next session") },
                previousSession: { [weak self] in self?.moves.append("previous session") }
            )
        }

        deinit {
            // the keyboard's moves hold it weakly
        }
    }
#endif
