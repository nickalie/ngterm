import UIKit

/// The simulator a golden is recorded on: the phone and the tablet lay text out a little differently (line heights
/// follow the device's idiom, whatever the traits say), so each keeps its own goldens, named after it.
internal enum SnapshotDevice {
    /// "phone" or "tablet": the suffix of the goldens recorded on this simulator.
    @MainActor internal static var current: String {
        UIDevice.current.userInterfaceIdiom == .pad ? "tablet" : "phone"
    }
}
