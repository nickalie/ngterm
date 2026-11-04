import Foundation
import Observation

/// The app's settings, kept as one JSON file in Application Support (`settings.json`).
///
/// It reads the file as ``MachineStore`` reads its own: a file that no longer decodes reads as the defaults and is
/// kept as `settings.json.bad`, and one that can't be read (or kept) is left alone until a change can read it (or keep
/// it). A change is written whole and atomically before ``settings`` shows it.
@preconcurrency
@MainActor
@Observable
public final class SettingsStore {
    /// The store's file name.
    public static let fileName = "settings.json"

    /// The settings.
    public private(set) var settings: AppSettings
    @ObservationIgnored private let file: JSONFile<AppSettings>
    // whether `settings` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            settings = try file.read() ?? AppSettings()
            loaded = true
        } catch {
            settings = AppSettings()
            loaded = false
        }
    }

    /// Changes the settings by `change`; the text size is held within ``AppSettings/textSizes``.
    public func update(_ change: (inout AppSettings) -> Void) throws(StoreError) {
        if !loaded {
            settings = try file.read() ?? AppSettings()
            loaded = true
        }
        var next = settings
        change(&next)
        next.textSize = AppSettings.clamped(next.textSize)
        try file.write(next)
        settings = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
