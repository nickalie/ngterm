/// The app's settings, as Settings shows them; each starts at its default.
///
/// The notification switches and the machines trusted with the clipboard are kept apart from these, as on Android.
public struct AppSettings: Codable, Hashable, Sendable {
    private enum CodingKeys: CodingKey {
        case agentChat
        case connectTimeout
        case extraKeys
        case font
        case keepScreenOn
        case palette
        case textSize
        case theme
    }

    /// The sizes the terminal's text may take, in points.
    public static let textSizes = 8...32
    /// The terminal's text size until the user picks another, in points.
    public static let defaultTextSize = 13
    /// The seconds a connect may be given to reach a machine, app-wide or for one machine
    /// (``StoredMachine/connectTimeout``).
    public static let connectTimeouts = 3...60
    /// The connect timeout until the user picks another, in seconds: the core's own default.
    public static let defaultConnectTimeout = 10

    /// The terminal's text size, in points, within ``textSizes``.
    public var textSize: Int
    /// The terminal's font.
    public var font: FontChoice
    /// The terminal's palette.
    public var palette: PaletteChoice
    /// Light or dark.
    public var theme: ThemeChoice
    /// The extra-keys bar.
    public var extraKeys: ExtraKeysChoice
    /// Whether the screen stays on while an attached session is open.
    public var keepScreenOn: Bool
    /// Whether a pane running Claude Code or Codex shows as a chat (Agent chat view); a session's More menu sets its
    /// own.
    public var agentChat: Bool
    /// The seconds a connect may take to reach a machine that sets none of its own, within ``connectTimeouts``.
    public var connectTimeout: Int

    /// The settings given, each defaulting to its default; the text size is held within ``textSizes``, and the connect
    /// timeout within ``connectTimeouts``.
    public init(
        textSize: Int = Self.defaultTextSize,
        font: FontChoice = .jetBrainsMono,
        palette: PaletteChoice = .theme,
        theme: ThemeChoice = .system,
        extraKeys: ExtraKeysChoice = .fit,
        keepScreenOn: Bool = false,
        agentChat: Bool = true,
        connectTimeout: Int = Self.defaultConnectTimeout
    ) {
        self.textSize = Self.clamped(textSize)
        self.font = font
        self.palette = palette
        self.theme = theme
        self.extraKeys = extraKeys
        self.keepScreenOn = keepScreenOn
        self.agentChat = agentChat
        self.connectTimeout = Self.clampedTimeout(connectTimeout)
    }

    /// The settings as stored: a value missing, or one this build doesn't know, reads as its default, and the text
    /// size and the connect timeout are held within their ranges.
    public init(from decoder: any Decoder) throws {
        let defaults = Self()
        let values = try decoder.container(keyedBy: CodingKeys.self)
        textSize = Self.clamped((try? values.decodeIfPresent(Int.self, forKey: .textSize)) ?? defaults.textSize)
        font = .stored(try? values.decodeIfPresent(String.self, forKey: .font)) ?? defaults.font
        palette = .stored(try? values.decodeIfPresent(String.self, forKey: .palette)) ?? defaults.palette
        theme = .stored(try? values.decodeIfPresent(String.self, forKey: .theme)) ?? defaults.theme
        extraKeys = .stored(try? values.decodeIfPresent(String.self, forKey: .extraKeys)) ?? defaults.extraKeys
        keepScreenOn = (try? values.decodeIfPresent(Bool.self, forKey: .keepScreenOn)) ?? defaults.keepScreenOn
        agentChat = (try? values.decodeIfPresent(Bool.self, forKey: .agentChat)) ?? defaults.agentChat
        connectTimeout = Self.clampedTimeout(
            (try? values.decodeIfPresent(Int.self, forKey: .connectTimeout)) ?? defaults.connectTimeout
        )
    }

    /// `size` held within ``textSizes``.
    public static func clamped(_ size: Int) -> Int {
        min(max(size, textSizes.lowerBound), textSizes.upperBound)
    }

    /// `seconds` held within ``connectTimeouts``.
    public static func clampedTimeout(_ seconds: Int) -> Int {
        min(max(seconds, connectTimeouts.lowerBound), connectTimeouts.upperBound)
    }

    /// Writes the settings, each choice by its case's name.
    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(textSize, forKey: .textSize)
        try values.encode(font.storedName, forKey: .font)
        try values.encode(palette.storedName, forKey: .palette)
        try values.encode(theme.storedName, forKey: .theme)
        try values.encode(extraKeys.storedName, forKey: .extraKeys)
        try values.encode(keepScreenOn, forKey: .keepScreenOn)
        try values.encode(agentChat, forKey: .agentChat)
        try values.encode(connectTimeout, forKey: .connectTimeout)
    }
}
