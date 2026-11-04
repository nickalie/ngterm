import CoreText
import SwiftUI
import UIKit

/// The bundled fonts, shared with the Android app: scripts/build-ios.sh copies them from its `res/font/`.
///
/// Atkinson Hyperlegible Mono is for what is compared character by character (hosts, fingerprints, key lines), and
/// JetBrains Mono is the terminal's face. The chrome is SF Pro.
public enum Fonts {
    /// The family of what is compared character by character.
    public static let monoFamily = "Atkinson Hyperlegible Mono"
    /// The regular instance of the mono family's variable font, by its PostScript name.
    ///
    /// The font's default instance is its extra light one, which a weight asked of the family doesn't move.
    public static let monoRegular = "AtkinsonHyperlegibleMono-ExtraLight_Regular"

    /// The terminal's faces, by PostScript name: regular, bold, italic and bold italic.
    public static let terminalFaces = TerminalFaces(
        regular: "JetBrainsMono-Regular",
        bold: "JetBrainsMono-Bold",
        italic: "JetBrainsMono-Italic",
        boldItalic: "JetBrainsMono-BoldItalic"
    )

    /// The bundled font files.
    public static let files = [
        "atkinson_hyperlegible_mono.ttf",
        "jetbrains_mono_regular.ttf",
        "jetbrains_mono_bold.ttf",
        "jetbrains_mono_italic.ttf",
        "jetbrains_mono_bold_italic.ttf",
    ]

    /// The fonts' licenses (the SIL Open Font License, with each font's copyright line), shipped beside them.
    public static let licenses = ["OFL-jetbrains-mono.txt", "OFL-atkinson-hyperlegible-mono.txt"]

    /// The files that could not be registered, by name: empty when every font is there to draw with.
    ///
    /// The first use registers them for the process.
    public static let unregistered: [String] = files.filter { file in
        guard let url = url(of: file) else {
            return true
        }
        var error: Unmanaged<CFError>?
        if CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) {
            return false
        }
        // already registered (a second process-wide registration) is as good as done
        let code = error.map { CFErrorGetCode($0.takeRetainedValue()) }
        return code != CTFontManagerError.alreadyRegistered.rawValue
    }

    // UIKit's names of SwiftUI's text styles, whose sizes it knows
    private static let uiKitStyles: [Font.TextStyle: UIFont.TextStyle] = [
        .largeTitle: .largeTitle,
        .title: .title1,
        .title2: .title2,
        .title3: .title3,
        .headline: .headline,
        .body: .body,
        .callout: .callout,
        .subheadline: .subheadline,
        .footnote: .footnote,
        .caption: .caption1,
        .caption2: .caption2,
    ]

    /// Registers the bundled fonts for the process, once; the app calls it at launch.
    public static func register() {
        _ = unregistered
    }

    /// The bundled file (a font or a license) named `name`.
    public static func url(of name: String) -> URL? {
        Bundle.module.url(forResource: name, withExtension: nil, subdirectory: "Fonts")
    }

    /// Atkinson Hyperlegible Mono at the size of `style`, scaling with Dynamic Type as `style` does.
    public static func mono(_ style: Font.TextStyle) -> Font {
        register()
        return Font.custom(monoRegular, size: defaultSize(of: style), relativeTo: style)
    }

    /// The size `style` has at the default Dynamic Type size (the large one), as UIKit gives it.
    public static func defaultSize(of style: Font.TextStyle) -> CGFloat {
        let large = UITraitCollection(preferredContentSizeCategory: .large)
        return UIFont.preferredFont(forTextStyle: uiKitStyles[style] ?? .body, compatibleWith: large).pointSize
    }
}
