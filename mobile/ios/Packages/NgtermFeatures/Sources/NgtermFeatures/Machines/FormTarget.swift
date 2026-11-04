/// What the machine form is open for: a new machine, or the one it edits.
internal enum FormTarget: Hashable, Identifiable {
    /// Add machine.
    case add
    /// Edit machine, by id.
    case edit(String)

    internal var id: String {
        switch self {
        case .add:
            "add"

        case .edit(let machineId):
            "edit \(machineId)"
        }
    }
}
