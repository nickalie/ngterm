import Foundation
import NgtermPlatform
import Testing

/// The settings' store: its JSON round trip, with Android's rules for values it doesn't know.
@MainActor
@Suite("the settings store")
internal struct SettingsStoreTests {
    @Test("no file is the defaults")
    internal func defaults() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let settings = SettingsStore(directory: directory.url).settings
        #expect(settings == AppSettings())
        #expect(settings.textSize == AppSettings.defaultTextSize)
        #expect(settings.font == .jetBrainsMono)
        #expect(settings.palette == .theme)
        #expect(settings.theme == .system)
        #expect(settings.extraKeys == .fit)
        #expect(!settings.keepScreenOn)
        #expect(settings.agentChat)
    }

    @Test("the settings round-trip through the file, each choice by its name")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let changed = AppSettings(
            textSize: 17,
            font: .systemMono,
            palette: .ghostty,
            theme: .dark,
            extraKeys: .twoLines,
            keepScreenOn: true,
            agentChat: false
        )
        try SettingsStore(directory: directory.url).update { $0 = changed }
        #expect(SettingsStore(directory: directory.url).settings == changed)
        let stored = try JSONSerialization.jsonObject(
            with: Data(contentsOf: directory.file(SettingsStore.fileName))
        )
        let fields = try #require(stored as? [String: Any])
        #expect(fields["font"] as? String == "systemMono")
        #expect(fields["extraKeys"] as? String == "twoLines")
        #expect(fields["textSize"] as? Int == 17)
        #expect(fields["agentChat"] as? Bool == false)
    }

    @Test("the text size stays within its range, whether set or stored")
    internal func textSizeClamped() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = SettingsStore(directory: directory.url)
        try store.update { $0.textSize = 99 }
        #expect(store.settings.textSize == AppSettings.textSizes.upperBound)
        try store.update { $0.textSize = 1 }
        #expect(store.settings.textSize == AppSettings.textSizes.lowerBound)
        try Data(#"{"textSize":400}"#.utf8).write(to: directory.file(SettingsStore.fileName))
        #expect(SettingsStore(directory: directory.url).settings.textSize == AppSettings.textSizes.upperBound)
    }

    @Test("a value this build doesn't know, or one missing, reads as its default; the others keep theirs")
    internal func unknownValues() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let json = #"{"font":"comicMono","palette":"drafting","theme":7,"keepScreenOn":true}"#
        try Data(json.utf8).write(to: directory.file(SettingsStore.fileName))
        let settings = SettingsStore(directory: directory.url).settings
        #expect(settings == AppSettings(palette: .drafting, keepScreenOn: true))
        #expect(try directory.names() == [SettingsStore.fileName])
    }

    @Test("every choice reads back from its stored name")
    internal func storedNames() {
        #expect(FontChoice.allCases.allSatisfy { FontChoice.stored($0.storedName) == $0 })
        #expect(PaletteChoice.allCases.allSatisfy { PaletteChoice.stored($0.storedName) == $0 })
        #expect(ThemeChoice.allCases.allSatisfy { ThemeChoice.stored($0.storedName) == $0 })
        #expect(ExtraKeysChoice.allCases.allSatisfy { ExtraKeysChoice.stored($0.storedName) == $0 })
        #expect(ThemeChoice.stored(nil) == nil)
        #expect(ThemeChoice.dark.storedName == "dark")
    }
}
