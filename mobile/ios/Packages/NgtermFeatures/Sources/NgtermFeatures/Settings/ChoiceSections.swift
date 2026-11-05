import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The settings that are one of a few values, each a section of its own: the terminal's font and colors (a palette
/// with its swatch), the app's appearance, and the extra keys' layout.
internal struct ChoiceSections: View {
    @Environment(\.colorScheme)
    private var colorScheme

    internal let model: SettingsModel

    internal var body: some View {
        Section {
            ChoicePicker(String(localized: "Font"), choices: SettingsChoices.fonts, selection: model.binding(\.font))
        } header: {
            FormHint(String(localized: "Font"), .header)
        }
        Section {
            ChoicePicker(
                String(localized: "Colors"),
                choices: SettingsChoices.palettes,
                selection: model.binding(\.palette)
            ) { choice in
                PaletteSwatch(palette: palette(of: choice))
            }
        } header: {
            FormHint(String(localized: "Colors"), .header)
        }
        Section {
            ChoicePicker(
                String(localized: "Appearance"),
                choices: SettingsChoices.themes,
                selection: model.binding(\.theme)
            )
        } header: {
            FormHint(String(localized: "Appearance"), .header)
        }
        Section {
            ChoicePicker(
                String(localized: "Extra keys"),
                choices: SettingsChoices.extraKeys,
                selection: model.binding(\.extraKeys)
            )
        } header: {
            FormHint(String(localized: "Extra keys"), .header)
        }
    }

    // the palette `choice` draws with in this appearance
    private func palette(of choice: PaletteChoice) -> TerminalPalette {
        var settings = model.settings
        settings.palette = choice
        return TerminalLook(settings, isDark: colorScheme == .dark).palette
    }
}
