import Foundation
import Observation

/// The machines the user added, in the order they were added, kept as one JSON file in Application Support
/// (`machines.json`).
///
/// It reads the file once, as it starts. A file that no longer decodes reads as no machines and is kept as
/// `machines.json.bad`. A file that can't be read (before the device's first unlock), or a corrupt one whose copy
/// can't be written, reads as no machines too, but is left alone: the next change reads it again, and fails rather
/// than write over it while it still can't be read or kept, and ``readIfUnread()`` tries again (a process the system
/// started for a background refresh before the first unlock, whose window opens later).
/// A change is written whole and atomically before ``machines`` shows it.
@preconcurrency
@MainActor
@Observable
public final class MachineStore {
    /// The store's file name.
    public static let fileName = "machines.json"

    /// The machines, in the order they were added.
    public private(set) var machines: [StoredMachine]
    @ObservationIgnored private let file: JSONFile<[StoredMachine]>
    // whether `machines` is what the file holds: false while the file is there but can't be read
    @ObservationIgnored private var loaded: Bool

    /// The store kept in `directory`: the app's Application Support unless a test gives its own.
    public init(directory: URL = .applicationSupportDirectory) {
        file = JSONFile(directory: directory, name: Self.fileName)
        do {
            machines = try file.read() ?? []
            loaded = true
        } catch {
            machines = []
            loaded = false
        }
    }

    /// Reads the file again if it couldn't be read before, so its machines show once it can be; a store that has read
    /// it keeps what it has.
    public func readIfUnread() {
        guard !loaded else {
            return
        }
        do {
            machines = try file.read() ?? []
            loaded = true
        } catch {
            // still unreadable: no machines, as before
        }
    }

    /// The machine `id`, if the user has it.
    public func machine(id: String) -> StoredMachine? {
        machines.first { $0.id == id }
    }

    /// Adds `machine`, or replaces the one with its id in place.
    public func save(_ machine: StoredMachine) throws(StoreError) {
        try change { list in
            if let index = list.firstIndex(where: { $0.id == machine.id }) {
                list[index] = machine
            } else {
                list.append(machine)
            }
        }
    }

    /// Puts `machine` back at `index` (an undone removal), or at the end when the list has since got shorter.
    public func restore(_ machine: StoredMachine, at index: Int) throws(StoreError) {
        try change { list in
            list.removeAll { $0.id == machine.id }
            list.insert(machine, at: min(max(index, 0), list.count))
        }
    }

    /// Points every machine at the key `keyId` (a rotated or imported key, once it is on the machines).
    public func useKey(_ keyId: String) throws(StoreError) {
        try change { list in
            for index in list.indices {
                list[index].keyId = keyId
            }
        }
    }

    /// Removes the machine `id`; nothing happens when there is none.
    public func delete(id: String) throws(StoreError) {
        try change { list in
            list.removeAll { $0.id == id }
        }
    }

    // applies `edit` to the machines and writes the result, then shows it; a file that couldn't be read is read first
    private func change(_ edit: (inout [StoredMachine]) -> Void) throws(StoreError) {
        if !loaded {
            machines = try file.read() ?? []
            loaded = true
        }
        var next = machines
        edit(&next)
        try file.write(next)
        machines = next
    }

    deinit {
        // nothing to release: the file is written as each change is made
    }
}
