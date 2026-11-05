import Foundation
import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import UIKit
import XCTest

/// The frame budget of a full-screen `cat` of a large file (mobile/android's `TerminalFrameTimeTest`).
///
/// The view fills the simulator's screen and takes frames that each scroll the whole grid by a few lines of a long
/// file, as a busy program's output does. The pane is a fake serving frames packed ahead as the core packs them, so
/// this times the app's half of each frame on the main thread: reading the buffer, taking its rows, laying out their
/// text and drawing every row's layer; the core's half is rows/tests.rs's `packing_a_full_screen_cat`, and compositing
/// is the render server's. XCTest's clock and CPU metrics measure runs of frames, and a percentile report of single
/// frames is printed (`TerminalFrames`). It checks only that frames were drawn: the numbers are for reading, and the
/// plan's Task 10 records them.
internal final class TerminalFrameTimeTests: XCTestCase {
    private static let textSize: CGFloat = 13
    private static let framesPerRun = 60
    private static let reportedFrames = 240
    private static let linesPerFrame = 7
    private static let budget: Duration = .microseconds(16_667)
    private static let alphabet = Array("abcdefghijklmnopqrstuvwxyz0123456789 .,;:(){}")
    private static let colors: [UInt32] = [
        0xFFE5_675C, 0xFF8C_C084, 0xFFE3_B65A, 0xFF6F_A3C8, 0xFFC0_8BB5, 0xFF5F_BFAA,
    ]

    // a line of the "file": 40 to 120 columns of text, a colored line number every fifth line
    private static func line(_ index: Int) -> [FakeRun] {
        let width = 40 + (index * 37) % 81
        let text = String((0..<width).map { alphabet[(index + $0 * 7) % alphabet.count] })
        guard index.isMultiple(of: 5) else {
            return [FakeRun(0, text)]
        }
        let number = String(repeating: " ", count: max(0, 6 - String(index).count)) + String(index)
        return [FakeRun(0, number, foreground: colors[index % colors.count]), FakeRun(7, text)]
    }

    // the grid's rows after `first` lines of the file scrolled by, each line a row, cut at the grid's edge
    private static func screen(first: Int, columns: Int, rows: Int, seq: UInt64) -> Data {
        let cursor = FakeCursor(column: 0, row: rows - 1)
        var writer = RowBufferWriter(columns: columns, rows: rows, seq: seq, full: false, cursor: cursor)
        for row in 0..<rows {
            let runs = line(first + row).compactMap { run -> FakeRun? in
                let room = columns - run.column
                guard room > 0 else {
                    return nil
                }
                return FakeRun(run.column, String(run.text.prefix(room)), foreground: run.foreground)
            }
            writer = writer.row(row, runs)
        }
        return writer.bytes()
    }

    private static func percentiles(_ times: [Duration]) -> String {
        let sorted = times.sorted()
        func at(_ percent: Int) -> String {
            let time = sorted[(sorted.count - 1) * percent / 100]
            let milliseconds = Double(time.components.seconds) * 1e3 + Double(time.components.attoseconds) / 1e15
            return String(format: "%.2f ms", milliseconds)
        }
        return "p50 \(at(50)), p90 \(at(90)), p99 \(at(99)), max \(at(100))"
    }

    @MainActor
    internal func testAFullScreenCatKeepsWithinTheFrameBudget() throws {
        // the pinned simulators' screens: an iPhone 17 at 3x, an iPad (A16) at 2x
        let tablet = UIDevice.current.userInterfaceIdiom == .pad
        let size = tablet ? CGSize(width: 820, height: 1_180) : CGSize(width: 402, height: 874)
        let scale: CGFloat = tablet ? 2 : 3
        let grid = try XCTUnwrap(TerminalFont(size: Self.textSize).grid(fitting: size, scale: scale))
        let columns = Int(grid.cols)
        let rows = Int(grid.rows)
        let frames = (0..<Self.framesPerRun).map { frame in
            Self.screen(first: frame * Self.linesPerFrame, columns: columns, rows: rows, seq: UInt64(frame + 2))
        }
        let reel = FrameReel(frames)
        let pane = FakePane { _ in reel.next() }
        let view = TerminalView(pane: pane, palette: .copper, textSize: Self.textSize)
        view.traitOverrides.displayScale = scale
        view.updateTraitsIfNeeded()
        view.frame = CGRect(origin: .zero, size: size)
        view.layoutIfNeeded()
        // a first pass draws the fonts' glyphs in once
        for _ in 0..<Self.framesPerRun {
            view.dirty()
            view.drawFrame()
            view.displayNow()
        }

        let options = XCTMeasureOptions()
        options.iterationCount = 5
        measure(metrics: [XCTClockMetric(), XCTCPUMetric()], options: options) {
            for _ in 0..<Self.framesPerRun {
                view.dirty()
                view.drawFrame()
                view.displayNow()
            }
        }

        // each frame in its two halves: reading it (decoding, taking its rows), then drawing the rows it changed
        let clock = ContinuousClock()
        let phases = (0..<Self.reportedFrames).map { _ in
            view.dirty()
            let read = clock.measure { view.drawFrame() }
            let draw = clock.measure { view.displayNow() }
            return (read: read, draw: draw)
        }
        let times = phases.map { $0.read + $0.draw }
        let over = times.filter { $0 > Self.budget }.count
        print(
            "TerminalFrames: \(times.count) frames at \(columns) x \(rows) "
                + "(\(Int(size.width)) x \(Int(size.height)) pt at \(Int(scale))x), "
                + "\(Self.linesPerFrame) lines each; per frame \(Self.percentiles(times)); "
                + "reading \(Self.percentiles(phases.map(\.read))); drawing \(Self.percentiles(phases.map(\.draw))); "
                + "\(over) over 16.7 ms; \(Self.framesPerRun) frames per measured run"
        )
        XCTAssertGreaterThan(reel.count, Self.framesPerRun)
        XCTAssertEqual(view.screen.rowCount, rows)
        XCTAssertFalse(view.screen.text(0).isEmpty)
    }

    deinit {
        // nothing to release
    }
}
