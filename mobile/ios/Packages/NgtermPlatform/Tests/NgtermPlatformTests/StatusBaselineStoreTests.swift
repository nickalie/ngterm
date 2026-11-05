import Foundation
import NgtermPlatform
import Testing

/// The status baseline's store: the statuses round-trip through `status-baseline.json`, for a refresh in a process
/// started afresh to read.
@MainActor
@Suite("the status baseline's store")
internal struct StatusBaselineStoreTests {
    @Test("the statuses round-trip through the file, without the machines that have none")
    internal func roundTrip() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        let store = StatusBaselineStore(directory: directory.url)
        #expect(store.statuses.isEmpty)
        try store.save(["m1": ["s1": "blocked", "s2": "active"], "m2": [:]])
        #expect(store.statuses == ["m1": ["s1": "blocked", "s2": "active"]])
        #expect(StatusBaselineStore(directory: directory.url).statuses == ["m1": ["s1": "blocked", "s2": "active"]])
        #expect(try directory.names() == [StatusBaselineStore.fileName])
    }

    @Test("a corrupt file reads as no statuses and is kept aside")
    internal func corruptFile() throws {
        let directory = try TemporaryDirectory()
        defer { directory.remove() }
        try Data("not json".utf8).write(to: directory.file(StatusBaselineStore.fileName))
        #expect(StatusBaselineStore(directory: directory.url).statuses.isEmpty)
        #expect(try directory.names() == [StatusBaselineStore.fileName + ".bad"])
    }
}
