import Foundation
import NgtermCore
import NgtermTerminal
import Testing

/// What a clipboard read's question shows of what it would hand over (Ghostty's macOS form).
@Suite("the clipboard read's summary")
internal struct ClipboardSummaryTests {
    @Test("UTF-8 text shows as itself, with the first image")
    internal func text() {
        let summary = ClipboardSummary([
            ClipboardPart(mime: "text/html", data: Data("<b>hi</b>".utf8)),
            ClipboardPart(mime: "image/png", data: ClipboardFixtures.redPNG),
            ClipboardPart(mime: "text/plain", data: Data("héllo".utf8)),
            ClipboardPart(mime: "image/jpeg", data: Data([1])),
        ])
        #expect(summary.text == "héllo")
        #expect(summary.image == ClipboardPart(mime: "image/png", data: ClipboardFixtures.redPNG))
    }

    @Test("without UTF-8 text each part says its type and size, an empty one included")
    internal func kinds() {
        let summary = ClipboardSummary([
            ClipboardPart(mime: "text/plain", data: Data([0xFF, 0xFE])),
            ClipboardPart(mime: "application/octet-stream", data: Data(count: 2_048)),
            ClipboardPart(mime: "text/html", data: Data()),
        ])
        #expect(summary.text == "text/plain (2 bytes)\napplication/octet-stream (2048 bytes)\ntext/html (0 bytes)")
        #expect(summary.image == nil)
    }

    @Test("no parts show no text")
    internal func empty() {
        #expect(ClipboardSummary([]).text == nil)
    }

    @Test("long text is cut on a character's edge and the rest counted")
    internal func bounded() {
        let kept = String(repeating: "a", count: ClipboardSummary.maxText - 1)
        let shown = ClipboardSummary.bounded(kept + "é" + String(repeating: "b", count: 1_500))
        #expect(shown.hasPrefix(kept + "\n"))
        #expect(shown.hasSuffix("… 1.5 kB more"))
        #expect(ClipboardSummary.bounded("short") == "short")
    }

    @Test("sizes read as Ghostty's dialog says them")
    internal func sizes() {
        #expect(ClipboardSummary.byteCount(1) == "1 byte")
        #expect(ClipboardSummary.byteCount(73) == "73 bytes")
        #expect(ClipboardSummary.byteCount(2_048) == "2.0 kB")
        #expect(ClipboardSummary.byteCount(1_500_000) == "1.5 MB")
    }
}
