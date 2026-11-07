import NgtermDesign
import SwiftUI

/// An edit's unified diff on a plate, in the terminal's face (mobile/android's `DiffView`): added lines in verdigris,
/// removed ones in oxide, the headers and hunks quiet (the headers bold), each keeping its `+` or `-` so the color is
/// never alone.
///
/// It scrolls sideways rather than wrapping; past ``DiffLines/limit`` lines it says how many more there are.
internal struct DiffView: View {
    internal let diff: String

    internal var body: some View {
        let parsed = DiffLines(diff)
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            ScrollView(.horizontal) {
                Text(Self.text(parsed))
                    .font(Typography.chatOutput)
                    .fixedSize(horizontal: true, vertical: false)
                    .textSelection(.enabled)
                    .padding(Spacing.medium)
            }
            .scrollIndicators(.hidden)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(ChatGrounds.plate, in: RoundedRectangle(cornerRadius: Radius.small, style: .continuous))
            if parsed.more > 0 {
                Text(
                    parsed.more == 1
                        ? String(localized: "1 more line, not shown")
                        : String(localized: "\(parsed.more) more lines, not shown")
                )
                .font(.footnote)
                .foregroundStyle(Palette.secondaryText.color)
            }
        }
    }

    private static func text(_ parsed: DiffLines) -> AttributedString {
        var text = AttributedString()
        for (index, line) in parsed.lines.enumerated() {
            var part = AttributedString(index > 0 ? "\n" + line.text : line.text)
            switch line.kind {
            case .file:
                part.foregroundColor = Palette.secondaryText.color
                part.font = Fonts.terminal(size: Typography.chatOutputSize, relativeTo: .caption, bold: true)

            case .hunk:
                part.foregroundColor = Palette.secondaryText.color

            case .added:
                part.foregroundColor = Palette.healthy.color

            case .removed:
                part.foregroundColor = Palette.failed.color

            case .context:
                part.foregroundColor = .primary
            }
            text += part
        }
        return text
    }
}
