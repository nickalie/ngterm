import SwiftUI

/// The type roles, on Dynamic Type text styles (SF Pro).
///
/// Every role scales with the user's text size, through the accessibility sizes.
public enum Typography {
    /// A machine's name in its row.
    public static let machineName = Font.headline
    /// A session's name in its row.
    public static let sessionName = Font.body.weight(.semibold)
    /// A row's second line: a session's context, what a machine's state means.
    public static let detail = Font.subheadline
    /// A status's word.
    public static let statusWord = Font.footnote.weight(.semibold)
    /// A banner's message.
    public static let banner = Font.subheadline.weight(.semibold)
    /// A banner's or a cover's explanation.
    public static let note = Font.footnote
    /// A cover's title.
    public static let coverTitle = Font.title3.weight(.semibold)

    /// A plate's label: "Public key", "Fingerprint".
    public static let plateLabel = Font.footnote.weight(.semibold)

    /// What is compared character by character, in a row: `user@host:port`.
    public static var address: Font {
        Fonts.mono(.footnote)
    }

    /// A plate's value: a key's line, a fingerprint, a command.
    public static var plateValue: Font {
        Fonts.mono(.callout)
    }
    /// What the terminal holds, in a chat, in JetBrains Mono at 13 pt.
    ///
    /// A tool's one line (its command, path, URL or query), a command in the `/` menu, a reply's code, an approval's
    /// command; it scales as the footnote does.
    public static var chatCode: Font {
        Fonts.terminal(size: chatCodeSize, relativeTo: .footnote)
    }
    /// A tool's output and a diff, in a chat: JetBrains Mono at 12 pt, scaling as the caption does.
    public static var chatOutput: Font {
        Fonts.terminal(size: chatOutputSize, relativeTo: .caption)
    }
    /// The size of ``chatCode`` at the default text size, in points.
    public static let chatCodeSize: CGFloat = 13
    /// The size of ``chatOutput`` at the default text size, in points.
    public static let chatOutputSize: CGFloat = 12
}
