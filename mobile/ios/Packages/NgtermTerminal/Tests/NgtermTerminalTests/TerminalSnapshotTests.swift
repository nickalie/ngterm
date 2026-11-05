import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import SnapshotTesting
import Testing
import UIKit

/// The terminal drawing known screens (``TerminalScenes``), as mobile/android's `TerminalScreenshotTest` does.
///
/// Colors, attributes and each line, wide characters, the cursor shapes and the selection, in the dark and light
/// palettes (Copper, Drafting) and Ghostty's, and at a larger text size. The terminal keeps its palette's ground
/// whatever the chrome's appearance, so the palette is the variant. Recorded at a display scale of 2, one set per
/// simulator.
@MainActor
@Suite("terminal snapshots")
internal struct TerminalSnapshotTests {
    private static let width: CGFloat = 390
    private static let sceneHeight: CGFloat = 230
    private static let shortHeight: CGFloat = 120
    private static let textSize: CGFloat = 13
    private static let scale: CGFloat = 2
    private static let cursorGap: CGFloat = 4
    nonisolated private static let themed: [TerminalPalette] = [.copper, .drafting]

    // a terminal over `pane`, laid out at `size`, its first frame read and drawn
    private static func terminal(
        _ pane: FakePane,
        palette: TerminalPalette,
        size: CGSize,
        textSize: CGFloat = textSize,
        focused: Bool = true
    ) -> TerminalView {
        let view = TerminalView(pane: pane, palette: palette, textSize: textSize)
        view.drawsFocused = focused
        view.traitOverrides.displayScale = scale
        view.updateTraitsIfNeeded()
        view.frame = CGRect(origin: .zero, size: size)
        view.layoutIfNeeded()
        view.drawFrame()
        view.displayNow()
        return view
    }

    private static func image(of view: UIView) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        return UIGraphicsImageRenderer(bounds: view.bounds, format: format).image { context in
            view.layer.render(in: context.cgContext)
        }
    }

    private func snapshot(
        _ view: UIView,
        named name: String,
        fileID: StaticString = #fileID,
        filePath: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line,
        column: UInt = #column
    ) {
        assertSnapshot(
            of: Self.image(of: view),
            as: .image(precision: 1, perceptualPrecision: 0.98),
            named: "\(name)-on-\(SnapshotDevice.current)",
            fileID: fileID,
            file: filePath,
            testName: testName,
            line: line,
            column: column
        )
    }

    private func scene(
        _ scene: @escaping TerminalScenes.Scene,
        named name: String,
        palette: TerminalPalette,
        height: CGFloat = sceneHeight,
        textSize: CGFloat = textSize,
        testName: String = #function,
        line: UInt = #line
    ) {
        let view = Self.terminal(
            TerminalScenes.pane(scene),
            palette: palette,
            size: CGSize(width: Self.width, height: height),
            textSize: textSize
        )
        snapshot(view, named: "\(name)-\(palette.id)", testName: testName, line: line)
    }

    @Test("colors", arguments: TerminalPalette.builtIn)
    internal func colors(palette: TerminalPalette) {
        scene(TerminalScenes.colors, named: "colors", palette: palette)
    }

    @Test("attributes and lines", arguments: themed)
    internal func attributes(palette: TerminalPalette) {
        scene(TerminalScenes.attributes, named: "attributes", palette: palette)
    }

    @Test("attributes and lines at twice the text size")
    internal func attributesLarge() {
        scene(
            TerminalScenes.attributes,
            named: "attributes-large",
            palette: .copper,
            height: Self.sceneHeight * 2,
            textSize: Self.textSize * 2
        )
    }

    @Test("wide characters, clusters and emoji", arguments: themed)
    internal func wide(palette: TerminalPalette) {
        scene(TerminalScenes.wide, named: "wide", palette: palette)
    }

    @Test("the selection", arguments: themed)
    internal func selection(palette: TerminalPalette) {
        scene(TerminalScenes.selection, named: "selection", palette: palette, height: Self.shortHeight)
    }

    /// The four shapes, the cursor on a wide character's second column, and a terminal out of focus (a hollow block).
    @Test("the cursor's shapes", arguments: themed)
    internal func cursors(palette: TerminalPalette) {
        let scenes: [(TerminalScenes.Scene, Bool)] = [
            (TerminalScenes.cursor(0), true),
            (TerminalScenes.cursor(1), true),
            (TerminalScenes.cursor(2), true),
            (TerminalScenes.cursor(3), true),
            (TerminalScenes.cursorOnWide, true),
            (TerminalScenes.cursor(0), false),
        ]
        let line = TerminalFont(size: Self.textSize).cellHeight
        let stack = UIView(
            frame: CGRect(
                x: 0,
                y: 0,
                width: Self.width,
                height: CGFloat(scenes.count) * (line + Self.cursorGap) - Self.cursorGap
            )
        )
        stack.backgroundColor = .black
        for (index, (scene, focused)) in scenes.enumerated() {
            let terminal = Self.terminal(
                TerminalScenes.pane(scene),
                palette: palette,
                size: CGSize(width: Self.width, height: line),
                focused: focused
            )
            terminal.frame.origin.y = CGFloat(index) * (line + Self.cursorGap)
            stack.addSubview(terminal)
        }
        snapshot(stack, named: "cursors-\(palette.id)")
    }
}
