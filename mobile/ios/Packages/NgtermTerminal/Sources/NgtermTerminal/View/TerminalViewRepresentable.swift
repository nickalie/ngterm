import NgtermCore
import NgtermDesign
import SwiftUI

/// A ``TerminalView`` in SwiftUI.
///
/// It shows the terminal of a pane in a palette, at a text size in a font, the pane's frame events arriving through a
/// ``TerminalFeed``. The view keeps its pane: give the representable an `.id` of the pane, so another pane gets a view
/// of its own.
public struct TerminalViewRepresentable: UIViewRepresentable {
    private let pane: any PaneClient
    private let feed: TerminalFeed
    private let palette: TerminalPalette
    private let textSize: CGFloat
    private let faces: TerminalFaces?
    private let focused: Bool

    /// The terminal of `pane`, told of new frames through `feed`.
    ///
    /// It draws in `palette` at `textSize` points in `faces` (the bundled JetBrains Mono by default; `nil` is the
    /// system's monospace), its cursor as it is when `focused`.
    public init(
        pane: any PaneClient,
        feed: TerminalFeed,
        palette: TerminalPalette,
        textSize: CGFloat,
        faces: TerminalFaces? = Fonts.terminalFaces,
        focused: Bool = true
    ) {
        self.pane = pane
        self.feed = feed
        self.palette = palette
        self.textSize = textSize
        self.faces = faces
        self.focused = focused
    }

    /// The terminal view, joined to the feed.
    public func makeUIView(context _: Context) -> TerminalView {
        let view = TerminalView(pane: pane, palette: palette, textSize: textSize, faces: faces)
        view.drawsFocused = focused
        feed.view = view
        return view
    }

    /// The text size it last gave the view: a pinch changes the view's own until a new one is given.
    public func makeCoordinator() -> TextSizeGiven {
        TextSizeGiven(textSize)
    }

    /// Takes the palette, the font and the focus as they are now, and the text size when it is a new one.
    public func updateUIView(_ view: TerminalView, context: Context) {
        view.palette = palette
        if context.coordinator.textSize != textSize {
            context.coordinator.textSize = textSize
            view.textSize = textSize
        }
        view.faces = faces
        view.drawsFocused = focused
        feed.view = view
    }
}
