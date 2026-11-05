import UIKit

/// The simulator a golden is recorded on: the phone and the tablet may lay text out a little differently, so each keeps
/// its own goldens, named after it (as NgtermDesign's and NgtermFeatures' snapshots do).
internal enum SnapshotDevice {
    /// "phone" or "tablet": the suffix of the goldens recorded on this simulator.
    @MainActor internal static var current: String {
        UIDevice.current.userInterfaceIdiom == .pad ? "tablet" : "phone"
    }
}
