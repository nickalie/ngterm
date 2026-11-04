/// The app's pinned host keys (trust on first use), held by the platform (the Keychain).
///
/// The core calls it on its actor thread while the actor waits for the answer, so it answers from what it holds
/// and never hops synchronously onto the main actor, which may itself be waiting on the core.
public protocol HostKeyStore: Sendable {
    /// The fingerprint pinned for the machine `machineId`, if any.
    func pinned(machineId: String) -> String?

    /// Pins `fingerprint` for the machine `machineId`, as the user accepted it.
    func pin(machineId: String, fingerprint: String)
}
