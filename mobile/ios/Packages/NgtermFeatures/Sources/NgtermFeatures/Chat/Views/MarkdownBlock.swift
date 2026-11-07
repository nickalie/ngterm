import Foundation

/// One block of a reply's Markdown, as ``MarkdownText`` draws it.
internal enum MarkdownBlock: Equatable {
    /// A fenced or indented code block, its text as written.
    case code(String)
    /// A heading of `level` (1 to 6).
    case heading(level: Int, AttributedString)
    /// A list item: its marker (`•`, `1.`), how deep it sits (0 at the top), and its text.
    case item(marker: String, depth: Int, AttributedString)
    /// A paragraph.
    case paragraph(AttributedString)
    /// A quote's paragraph.
    case quote(AttributedString)
    /// A thematic break.
    case rule
    /// A table: its rows of cells, the header's first.
    case table(rows: [[AttributedString]], hasHeader: Bool)

    /// An unordered list item's marker.
    internal static let bullet = "•"
    /// The second heading level, the last drawn at a title size.
    internal static let secondLevel = 2

    /// A reply's blocks, in order.
    ///
    /// The Markdown is parsed by Foundation (CommonMark with GitHub's tables), each block one run of the same
    /// presentation intent. A reply that doesn't parse is one paragraph of its text.
    internal static func blocks(of markdown: String) -> [Self] {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: false,
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        guard let parsed = try? AttributedString(markdown: markdown, options: options) else {
            return [.paragraph(AttributedString(markdown))]
        }
        var reader = MarkdownBlockReader()
        for (intent, range) in parsed.runs[\.presentationIntent] {
            reader.take(AttributedString(parsed[range]), intent: intent)
        }
        return reader.done()
    }

    /// The block with `text`, a later run of its own intent, after its text.
    internal func extended(with text: AttributedString) -> Self {
        switch self {
        case .paragraph(let before):
            .paragraph(before + text)

        case .heading(let level, let before):
            .heading(level: level, before + text)

        case .code(let before):
            .code(before + String(text.characters))

        case .item(let marker, let depth, let before):
            .item(marker: marker, depth: depth, before + text)

        case .quote(let before):
            .quote(before + text)

        case .table, .rule:
            self
        }
    }
}
