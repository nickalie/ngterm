import NgtermDesign
import SnapshotTesting
import SwiftUI

/// Records a screen as a golden: themed, a phone's size, in a ``ScreenVariant``.
///
/// Public, so its check can take the caller's test name and place by default (SwiftLint keeps defaults out of
/// internal functions).
@preconcurrency
@MainActor
public enum ScreenSnapshot {
    /// What a golden's buttons do: nothing.
    public static let ignore: @MainActor () -> Void = {
        // a golden presses nothing
    }

    /// What a golden's buttons that pass a choice do: nothing.
    public static let ignoreChoice: @MainActor (Bool) -> Void = { _ in
        // a golden presses nothing
    }

    // how alike a pixel must be to its golden's: text antialiasing differs a little between runs
    private static let perceptualPrecision: Float = 0.98

    /// Checks `view` against the golden `name` in `variant`, recorded on this simulator.
    public static func check(
        _ view: some View,
        named name: String,
        variant: ScreenVariant,
        fileID: StaticString = #fileID,
        filePath: StaticString = #filePath,
        testName: String = #function,
        line: UInt = #line,
        column: UInt = #column
    ) {
        let themed =
            view
            .ngtermTheme()
            .environment(\.colorScheme, variant.colorScheme)
            .environment(\.dynamicTypeSize, variant.typeSize)
            .environment(\.horizontalSizeClass, .compact)
        assertSnapshot(
            of: themed,
            as: .image(
                precision: 1,
                perceptualPrecision: perceptualPrecision,
                layout: .fixed(width: ScreenVariant.size.width, height: ScreenVariant.size.height),
                traits: variant.traits
            ),
            named: "\(name)-\(variant)-on-\(SnapshotDevice.current)",
            fileID: fileID,
            file: filePath,
            testName: testName,
            line: line,
            column: column
        )
    }
}
