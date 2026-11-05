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
    private var keyboard: TerminalKeyboard?
    private var select: (@MainActor () -> Void)?
    private var swipe: (@MainActor (Bool) -> Void)?
    private var readsAloud = false

    /// The terminal of `pane`, told of new frames through `feed`.
    ///
    /// It draws in `palette` at `textSize` points in `faces` (the bundled JetBrains Mono by default; `nil` is the
    /// system's monospace), its cursor as it is when `focused`. It takes its keys through a keyboard of its own unless
    /// given one (``keyboard(_:)``).
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

    /// The same terminal, taking its keys through `keyboard`, which the terminals on screen share.
    public func keyboard(_ keyboard: TerminalKeyboard) -> Self {
        var copy = self
        copy.keyboard = keyboard
        return copy
    }

    /// The same terminal, running `action` as a touch or a click lands on it, or it takes the focus
    /// (``TerminalView/onSelect``).
    @preconcurrency
    public func onSelect(_ action: @escaping @MainActor () -> Void) -> Self {
        var copy = self
        copy.select = action
        return copy
    }

    /// The same terminal, a finger's sideways swipe going to `action` (``TerminalView/onSwipe``); with none, a drag
    /// only scrolls.
    @preconcurrency
    public func onSwipe(_ action: (@MainActor (Bool) -> Void)?) -> Self {
        var copy = self
        copy.swipe = action
        return copy
    }

    /// The same terminal, saying its cursor's line to VoiceOver as it changes when `reads` (the selected pane).
    public func readsAloud(_ reads: Bool) -> Self {
        var copy = self
        copy.readsAloud = reads
        return copy
    }

    /// The terminal view, joined to the feed.
    public func makeUIView(context _: Context) -> TerminalView {
        let view = TerminalView(pane: pane, palette: palette, textSize: textSize, faces: faces, keyboard: keyboard)
        take(into: view)
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
            view.defaultTextSize = textSize
        }
        view.faces = faces
        take(into: view)
    }

    // what changes with every update: the hooks, then the focus, which may take the focus handed over
    private func take(into view: TerminalView) {
        view.onSelect = select
        view.onSwipe = swipe
        view.readsAloud = readsAloud
        view.drawsFocused = focused
        feed.view = view
    }
}
