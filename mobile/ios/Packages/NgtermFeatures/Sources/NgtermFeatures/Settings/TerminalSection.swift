import NgtermDesign
import NgtermPlatform
import SwiftUI

/// The terminal: a live sample in the chosen font, size and colors, the text size, and keeping the screen on.
internal struct TerminalSection: View {
    @Environment(\.colorScheme)
    private var colorScheme

    internal let model: SettingsModel

    internal var body: some View {
        let settings = model.settings
        let isDark = colorScheme == .dark
        Section {
            TerminalPreview(
                palette: TerminalLook(settings, isDark: isDark).palette,
                paletteName: SettingsChoices.name(of: settings.palette, isDark: isDark),
                font: settings.font,
                textSize: settings.textSize
            )
            textSize(settings.textSize)
            Toggle(isOn: model.binding(\.keepScreenOn)) {
                ChoiceLabel(
                    String(localized: "Keep the screen on"),
                    note: String(localized: "While an attached session is open")
                )
            }
            .accessibilityIdentifier("keep-screen-on")
        } header: {
            FormHint(String(localized: "Terminal"), .header)
        }
    }

    // the text size: its value in points, a slider of whole points over the terminal's sizes, and what it sets
    private func textSize(_ size: Int) -> some View {
        let label = String(localized: "Text size")
        let range = AppSettings.textSizes
        return VStack(alignment: .leading, spacing: Spacing.small) {
            HStack(alignment: .firstTextBaseline) {
                Text(label)
                Spacer()
                Text("\(size) pt")
                    .font(Typography.address)
                    .foregroundStyle(Palette.secondaryText.color)
            }
            .accessibilityElement(children: .combine)
            Slider(
                value: Binding {
                    Double(size)
                } set: { picked in
                    let whole = Int(picked.rounded())
                    if whole != model.settings.textSize {
                        model.change { $0.textSize = whole }
                    }
                },
                in: Double(range.lowerBound)...Double(range.upperBound),
                step: 1
            ) {
                Text(label)
            }
            .accessibilityValue(Text("\(size) points"))
            .accessibilityIdentifier("text-size")
            FormHint(
                String(
                    localized: """
                        Every pane starts at this size. A pinch or ⌘+ changes one pane until you set it here again.
                        """
                ),
                .note
            )
        }
        .padding(.vertical, Spacing.xSmall)
    }
}
