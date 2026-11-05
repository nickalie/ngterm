import NgtermDesign
import NgtermPlatform
import SwiftUI
import UIKit

/// A few lines of a shell as the terminal would draw them (mobile/android's `TerminalPreview`).
///
/// They show in `palette`, `font` and `textSize` (whole points, as the terminal's own size: it doesn't follow Dynamic
/// Type): a prompt, a listing, `git status` with one line selected, and the cursor. Lines wider than the plate are
/// cut, as a terminal's are. VoiceOver reads it as an image of what it shows.
internal struct TerminalPreview: View {
    // the ANSI colors the sample uses, by index
    private static let red = 1
    private static let green = 2
    private static let yellow = 3
    private static let blue = 4
    private static let magenta = 5
    private static let cyan = 6
    // the sample's lines
    private static let lines = 7

    internal let palette: TerminalPalette
    internal let paletteName: String
    internal let font: FontChoice
    internal let textSize: Int

    internal var body: some View {
        // drawn, not laid out as text: the sample's colors are the program's, which the audit would hold to 4.5:1 as
        // the app's text, and VoiceOver reads the plate as one image of what it shows
        let drawn = Text(sample).font(face(bold: false)).foregroundStyle(palette.foreground.color)
        Canvas { context, _ in
            context.draw(drawn, at: .zero, anchor: .topLeading)
        }
        .frame(height: lineHeight * CGFloat(Self.lines))
        .clipped()
        .padding(Spacing.medium)
        .background(palette.background.color, in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(.isImage)
        .accessibilityLabel(Text("A sample of the terminal in \(paletteName) at \(textSize) points"))
        .accessibilityIdentifier("terminal-preview")
    }

    // a line of the face at the terminal's size, as UIKit lays it out
    private var lineHeight: CGFloat {
        let size = CGFloat(textSize)
        let system = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        let face = font == .jetBrainsMono ? UIFont(name: Fonts.terminalFaces.regular, size: size) : nil
        return ceil((face ?? system).lineHeight)
    }

    private var sample: AttributedString {
        var text = ansi("nick@build-01", Self.green) + plain(" ") + ansi("~/docs-site", Self.blue)
        text += plain(" ") + ansi("(main)", Self.magenta) + plain("\n$ ls\n")
        text += bold("site/", Self.blue) + plain("  README.md  ") + bold("build.sh", Self.green)
        text += plain("  ") + ansi("latest", Self.cyan) + plain("\n$ git status --short\n")
        var selected = plain("site/install.md")
        selected.backgroundColor = palette.selectionBackground.color
        text += ansi(" M", Self.red) + plain(" ") + selected + plain("\n")
        text += ansi("??", Self.yellow) + plain(" site/ios.md\n$ ")
        var cursor = plain(" ")
        cursor.backgroundColor = palette.cursor.color
        return text + cursor
    }

    // the face at the terminal's size: JetBrains Mono's regular or bold, or the system's monospace
    private func face(bold isBold: Bool) -> Font {
        let size = CGFloat(textSize)
        guard font == .jetBrainsMono else {
            return .system(size: size, weight: isBold ? .bold : .regular, design: .monospaced)
        }
        let faces = Fonts.terminalFaces
        return .custom(isBold ? faces.bold : faces.regular, fixedSize: size)
    }

    private func plain(_ string: String) -> AttributedString {
        AttributedString(string)
    }

    private func ansi(_ string: String, _ index: Int) -> AttributedString {
        var run = AttributedString(string)
        run.foregroundColor = palette.ansi[index].color
        return run
    }

    private func bold(_ string: String, _ index: Int) -> AttributedString {
        var run = ansi(string, index)
        run.font = face(bold: true)
        return run
    }
}
