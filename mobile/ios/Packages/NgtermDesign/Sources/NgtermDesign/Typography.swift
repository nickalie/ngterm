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
}
