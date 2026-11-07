import NgtermDesign
import SwiftUI

/// A reply, full width on the ground (mobile/android's `ReplyText`), its Markdown drawn as the app's text.
///
/// Paragraphs and lists in the body size, headings at the title sizes (never the large title: a reply is not a page),
/// links in the tint and underlined, quotes in `secondaryText` behind a rule. Code blocks are the terminal's face at 13
/// pt on a plate, scrolling sideways, never highlighted; inline code the same face. Images aren't fetched: their words
/// stand in. The text can be selected to copy.
internal struct MarkdownText: View {
    // inline code's size beside the body's text
    private static let inlineCodePoints: CGFloat = 15

    private let blocks: [MarkdownBlock]

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                view(block)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .textSelection(.enabled)
    }

    /// The reply `markdown`, parsed as it is first drawn.
    internal init(_ markdown: String) {
        blocks = MarkdownBlock.blocks(of: markdown)
    }

    private static func styled(_ text: AttributedString) -> AttributedString {
        var styled = text
        for run in styled.runs where run.inlinePresentationIntent?.contains(.code) == true {
            styled[run.range].font = Fonts.terminal(size: inlineCodePoints, relativeTo: .body)
        }
        for run in styled.runs where run.link != nil {
            styled[run.range].foregroundColor = Palette.tint.color
            styled[run.range].underlineStyle = .single
        }
        return styled
    }

    // a heading's font: the title sizes for the first two levels, the headline for the rest
    private static func headingFont(_ level: Int) -> Font {
        switch level {
        case ...1:
            .title2.weight(.semibold)

        case MarkdownBlock.secondLevel:
            .title3.weight(.semibold)

        default:
            .headline
        }
    }

    @ViewBuilder
    private func view(_ block: MarkdownBlock) -> some View {
        switch block {
        case .paragraph(let text):
            Text(Self.styled(text))
                .font(.body)
                .fixedSize(horizontal: false, vertical: true)

        case .heading(let level, let text):
            Text(Self.styled(text))
                .font(Self.headingFont(level))
                .accessibilityAddTraits(.isHeader)
                .fixedSize(horizontal: false, vertical: true)

        case .code(let code):
            ScrollView(.horizontal) {
                Text(verbatim: code)
                    .font(Typography.chatCode)
                    .fixedSize(horizontal: true, vertical: false)
                    .padding(Spacing.medium)
            }
            .scrollIndicators(.hidden)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(ChatGrounds.plate, in: RoundedRectangle(cornerRadius: Radius.small, style: .continuous))

        case .item(let marker, let depth, let text):
            HStack(alignment: .firstTextBaseline, spacing: Spacing.small) {
                Text(verbatim: marker)
                    .monospacedDigit()
                    .accessibilityHidden(marker == MarkdownBlock.bullet)
                Text(Self.styled(text))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .font(.body)
            .padding(.leading, CGFloat(depth) * Spacing.large)

        case .quote(let text):
            HStack(spacing: Spacing.small) {
                Capsule()
                    .fill(Color(uiColor: .separator))
                    .frame(width: ChatStyle.ruleWidth)
                    .accessibilityHidden(true)
                Text(Self.styled(text))
                    .font(.body)
                    .foregroundStyle(Palette.secondaryText.color)
                    .fixedSize(horizontal: false, vertical: true)
            }

        case .table(let rows, let hasHeader):
            table(rows, hasHeader: hasHeader)

        case .rule:
            Divider()
        }
    }

    private func table(_ rows: [[AttributedString]], hasHeader: Bool) -> some View {
        ScrollView(.horizontal) {
            Grid(alignment: .leading, horizontalSpacing: Spacing.medium, verticalSpacing: Spacing.xSmall) {
                ForEach(Array(rows.enumerated()), id: \.offset) { index, cells in
                    GridRow {
                        ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                            Text(Self.styled(cell))
                                .font(index == 0 && hasHeader ? .subheadline.weight(.semibold) : .subheadline)
                        }
                    }
                }
            }
            .padding(Spacing.medium)
        }
        .scrollIndicators(.hidden)
        .background(ChatGrounds.plate, in: RoundedRectangle(cornerRadius: Radius.small, style: .continuous))
    }
}
