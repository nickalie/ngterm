import NgtermPlatform

/// The words of Settings' choices, in the order they are offered (mobile/android's).
internal enum SettingsChoices {
    /// The fonts, with what each is.
    internal static let fonts: [SettingsChoice<FontChoice>] = [
        SettingsChoice(
            .jetBrainsMono,
            String(localized: "JetBrains Mono"),
            note: String(localized: "The face desktop ngterm and Ghostty draw with")
        ),
        SettingsChoice(
            .systemMono,
            String(localized: "System monospace"),
            note: String(localized: "SF Mono, the system's fixed-width font")
        ),
    ]

    /// The palettes, the theme's first, with what each is.
    internal static let palettes: [SettingsChoice<PaletteChoice>] = [
        SettingsChoice(
            .theme,
            String(localized: "Match the theme"),
            note: String(localized: "Copper when dark, Drafting when light")
        ),
        SettingsChoice(.copper, String(localized: "Copper"), note: String(localized: "Graphite ground, copper cursor")),
        SettingsChoice(.drafting, String(localized: "Drafting"), note: String(localized: "Paper ground, ink text")),
        SettingsChoice(
            .ghostty,
            String(localized: "Ghostty"),
            note: String(localized: "Desktop ngterm's starting colors")
        ),
    ]

    /// The appearances; only the system's needs a word more.
    internal static let themes: [SettingsChoice<ThemeChoice>] = [
        SettingsChoice(.system, String(localized: "System"), note: String(localized: "Light or dark as the device is")),
        SettingsChoice(.light, String(localized: "Light"), note: nil),
        SettingsChoice(.dark, String(localized: "Dark"), note: nil),
    ]

    /// The extra keys' layouts, with what each does.
    internal static let extraKeys: [SettingsChoice<ExtraKeysChoice>] = [
        SettingsChoice(
            .fit,
            String(localized: "Fit to the width"),
            note: String(localized: "Two lines on a phone held upright, one line when every key gets room")
        ),
        SettingsChoice(
            .twoLines,
            String(localized: "Always two lines"),
            note: String(localized: "Wider keys on a tablet or held sideways")
        ),
        SettingsChoice(
            .hidden,
            String(localized: "Hidden"),
            note: String(
                localized: """
                    Only the soft keyboard, which on an iPhone has no key that closes it. \
                    A hardware keyboard always hides the bar.
                    """
            )
        ),
    ]

    /// The notification kinds, with what each tells.
    internal static let notices: [SettingsChoice<NoticeKind>] = [
        SettingsChoice(
            .waiting,
            String(localized: "Waiting for you"),
            note: String(localized: "An agent stopped and waits for your answer")
        ),
        SettingsChoice(.done, String(localized: "Finished"), note: String(localized: "An agent finished its work")),
        SettingsChoice(
            .notices,
            String(localized: "Agent notifications"),
            note: String(localized: "What an agent sends with ngtermctl notify")
        ),
        SettingsChoice(
            .errors,
            String(localized: "Lost machines"),
            note: String(localized: "A machine with attached sessions can't be reached")
        ),
    ]

    /// The name of `choice`, as the preview says it.
    internal static func name(of choice: PaletteChoice) -> String {
        palettes.first { $0.value == choice }?.title ?? ""
    }
}
