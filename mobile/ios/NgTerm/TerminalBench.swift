#if DEBUG
    import NgtermDesign
    import NgtermPlatform
    import NgtermTerminal
    import SwiftUI

    /// A terminal on a fake pane alone on screen (the fake core's `-ngterm-fake-terminal`), for the keyboard's UI
    /// tests until the session view shows terminals: over it, what the pane was typed and pressed, the moves the chords
    /// ran and the pane's grid, read again a few times a second.
    internal struct TerminalBench: View {
        private static let beat: TimeInterval = 0.2

        @State private var model = TerminalBenchModel()

        internal var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                TimelineView(.periodic(from: .now, by: Self.beat)) { _ in
                    VStack(alignment: .leading) {
                        Text(verbatim: model.typed).accessibilityIdentifier("bench-typed")
                        Text(verbatim: model.keys).accessibilityIdentifier("bench-keys")
                        Text(verbatim: model.moves.joined(separator: ", ")).accessibilityIdentifier("bench-moves")
                        Text(verbatim: model.grid).accessibilityIdentifier("bench-grid")
                    }
                    .font(.footnote)
                    .padding()
                }
                TerminalViewRepresentable(
                    pane: model.pane,
                    feed: model.feed,
                    palette: .copper,
                    textSize: CGFloat(AppSettings.defaultTextSize)
                )
                .keyboard(model.keyboard)
            }
            .ngtermTheme()
        }
    }
#endif
