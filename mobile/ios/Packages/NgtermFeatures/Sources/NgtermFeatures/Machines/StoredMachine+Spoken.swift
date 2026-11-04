import NgtermPlatform

extension StoredMachine {
    /// Its address as VoiceOver says it: "nick at build-01.lan, port 22", not the symbols of `user@host:port`.
    internal var spokenAddress: String {
        String(localized: "\(user) at \(host), port \(String(port))")
    }
}
