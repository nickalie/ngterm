/// A session as a window shows it: its machine and its id there.
///
/// It is the value of the app's session window group, so SwiftUI keeps one window per value and focuses that window
/// when it is opened again.
public struct SessionWindowID: Codable, Hashable, Sendable {
    /// The machine's id in the app's store.
    public let machineID: String
    /// The session's id on the machine.
    public let sessionID: String

    /// The session `sessionID` on the machine `machineID`.
    public init(machineID: String, sessionID: String) {
        self.machineID = machineID
        self.sessionID = sessionID
    }
}
