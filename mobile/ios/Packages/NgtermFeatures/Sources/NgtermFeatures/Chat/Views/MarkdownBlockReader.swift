import Foundation

/// Folds a reply's runs into ``MarkdownBlock``s: runs of one intent are one block, a table's cells gather into its
/// rows.
internal struct MarkdownBlockReader {
    // where a run sits in a table: the table, its row (nil: the header) and its column
    private struct Cell {
        let table: Int
        let row: Int?
        let column: Int
    }

    // a table being read: its identity, its rows so far, whether it has a header, and the row read last
    private struct Table {
        let identity: Int
        var rows: [[AttributedString]] = []
        let hasHeader: Bool
        var row: Int?
    }

    private var blocks: [MarkdownBlock] = []
    private var lastIdentity: Int?
    private var table: Table?

    private static func block(_ text: AttributedString, intent: PresentationIntent) -> MarkdownBlock {
        var depth = -1
        var marker: String?
        var isOrdered = false
        var isQuote = false
        // the components run from the innermost out: the first list met is the item's own
        for component in intent.components {
            switch component.kind {
            case .listItem(let ordinal):
                marker = marker ?? String(ordinal)

            case .orderedList:
                isOrdered = isOrdered || depth < 0
                depth += 1

            case .unorderedList:
                depth += 1

            case .blockQuote:
                isQuote = true

            default:
                break
            }
        }
        switch intent.components.first?.kind {
        case .header(let level):
            return .heading(level: level, text)

        case .codeBlock:
            return .code(String(text.characters).trimmingCharacters(in: .newlines))

        case .thematicBreak:
            return .rule

        default:
            if let marker {
                return .item(marker: isOrdered ? "\(marker)." : MarkdownBlock.bullet, depth: max(depth, 0), text)
            }
            return isQuote ? .quote(text) : .paragraph(text)
        }
    }

    // the cell a run is in, or nil for a run outside any table
    private static func cell(_ intent: PresentationIntent) -> Cell? {
        var column: Int?
        var row: Int?
        var isHeader = false
        for component in intent.components {
            switch component.kind {
            case .tableCell(let index):
                column = index

            case .tableRow(let index):
                row = index

            case .tableHeaderRow:
                isHeader = true

            case .table:
                return column.map { Cell(table: component.identity, row: isHeader ? nil : row, column: $0) }

            default:
                break
            }
        }
        return nil
    }

    /// Takes the run `text` of `intent`.
    internal mutating func take(_ text: AttributedString, intent: PresentationIntent?) {
        guard let intent, let first = intent.components.first else {
            append(.paragraph(text), identity: nil)
            return
        }
        if let cell = Self.cell(intent) {
            add(text, to: cell)
            return
        }
        endTable()
        // a block's later runs carry the same intent: they go on its text
        if first.identity == lastIdentity, let last = blocks.popLast() {
            blocks.append(last.extended(with: text))
            return
        }
        append(Self.block(text, intent: intent), identity: first.identity)
    }

    /// The blocks read.
    internal mutating func done() -> [MarkdownBlock] {
        endTable()
        return blocks
    }

    private mutating func add(_ text: AttributedString, to cell: Cell) {
        if table?.identity != cell.table {
            endTable()
            table = Table(identity: cell.table, hasHeader: cell.row == nil)
        }
        guard var now = table else {
            return
        }
        let rowKey = cell.row ?? -1
        if now.row != rowKey || now.rows.isEmpty {
            now.rows.append([])
            now.row = rowKey
        }
        var cells = now.rows.removeLast()
        if cells.count > cell.column {
            cells[cell.column] += text
        } else {
            cells.append(contentsOf: Array(repeating: AttributedString(), count: cell.column - cells.count))
            cells.append(text)
        }
        now.rows.append(cells)
        table = now
    }

    private mutating func endTable() {
        guard let now = table else {
            return
        }
        table = nil
        append(.table(rows: now.rows, hasHeader: now.hasHeader), identity: nil)
    }

    private mutating func append(_ block: MarkdownBlock, identity: Int?) {
        blocks.append(block)
        lastIdentity = identity
    }
}
