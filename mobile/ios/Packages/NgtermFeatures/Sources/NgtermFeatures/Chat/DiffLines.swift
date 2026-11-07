/// A unified diff's first lines (mobile/android's `diffLines`), and how many more it has.
///
/// A file's header lines until its first hunk, then the hunks' lines by their first character. A `---` or `+++` inside
/// a hunk is a removed or added line; once a hunk has all the lines its `@@` counts (or, with no counts, at a `---`
/// line right over a `+++` one) a new file's header starts, as in the core's diffs that join one per edit or file.
public struct DiffLines: Equatable, Sendable {
    // where a unified diff's reading stands: in a file's header, or in a hunk with the old and new lines it has left
    // (nil when its `@@` gave no counts)
    private struct DiffReader {
        private var header = true
        private var left: (old: Int, new: Int)?

        private static func kind(_ line: String, header: Bool) -> DiffLineKind {
            if line.hasPrefix("@@") {
                return .hunk
            }
            if header {
                return .file
            }
            if line.hasPrefix("+") {
                return .added
            }
            return line.hasPrefix("-") ? .removed : .context
        }

        // the old and new counts of a hunk's `@@ -a,b +c,d @@` line (1 when left out), or nil when it isn't one
        private static func counts(_ line: String) -> (old: Int, new: Int)? {
            let parts = line.split(separator: " ")
            let ranges = parts.dropFirst()
            guard parts.first == "@@", let oldRange = ranges.first, let newRange = ranges.dropFirst().first,
                oldRange.hasPrefix("-"), newRange.hasPrefix("+")
            else {
                return nil
            }
            func count(_ range: Substring) -> Int? {
                let fields = range.dropFirst().split(separator: ",", omittingEmptySubsequences: false)
                guard let first = fields.first, Int(first) != nil else {
                    return nil
                }
                return fields.dropFirst().first.map { Int($0) } ?? 1
            }
            guard let old = count(oldRange), let new = count(newRange) else {
                return nil
            }
            return (old, new)
        }

        mutating func kind(_ line: String, next: String?) -> DiffLineKind {
            if line.hasPrefix("diff ") {
                header = true
            } else if line.hasPrefix("@@") {
                header = false
                left = Self.counts(line)
            } else if !header {
                header = hunkOver(line, next: next)
            }
            return Self.kind(line, header: header)
        }

        // whether `line` starts a new file's header rather than going on with the hunk; counts it against the hunk if
        // not
        private mutating func hunkOver(_ line: String, next: String?) -> Bool {
            guard var counts = left else {
                return line.hasPrefix("--- ") && next?.hasPrefix("+++ ") == true
            }
            // "\ No newline at end of file" follows the line it is about, inside the hunk
            if line.hasPrefix("\\") {
                return false
            }
            if counts.old <= 0, counts.new <= 0 {
                return true
            }
            switch line.first {
            case "-":
                counts.old -= 1

            case "+":
                counts.new -= 1

            default:
                counts.old -= 1
                counts.new -= 1
            }
            left = counts
            return false
        }
    }

    /// The most lines a diff shows: the rest is the terminal's to show.
    public static let limit = 400

    /// The lines shown.
    public let lines: [DiffLine]
    /// How many more the diff has.
    public let more: Int

    /// The first `limit` lines of `diff`, each with what it is.
    public init(_ diff: String, limit: Int = Self.limit) {
        var trimmed = Substring(diff)
        while trimmed.hasSuffix("\n") {
            trimmed = trimmed.dropLast()
        }
        let all = trimmed.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var reader = DiffReader()
        lines = all.prefix(limit).enumerated().map { index, line in
            let next = index + 1 < all.count ? all[index + 1] : nil
            return DiffLine(kind: reader.kind(line, next: next), text: line)
        }
        more = max(all.count - limit, 0)
    }
}
