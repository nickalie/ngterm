import NgtermCore
import Testing

/// The words the app shows for the core's failures: a call's, and a key's that couldn't be made or imported.
@Suite("the core's failures in words")
internal struct ErrorReasonTests {
    @Test(
        "a call's failure reads as the core gives it",
        arguments: [
            (MobileError.Unreachable(reason: "connection refused"), "connection refused"),
            (MobileError.Remote(reason: "no session s1"), "no session s1"),
            (MobileError.Cancelled, "the call was cancelled"),
            (MobileError.Stopped, "the core has stopped"),
            (MobileError.NotConnected(machineId: "m1"), "m1 is not connected"),
            (MobileError.NotListed(sessionId: "s1"), "s1 is not in the machine's last listing"),
        ]
    )
    internal func callFailure(error: MobileError, reason: String) {
        #expect(error.reason == reason)
    }

    @Test(
        "a key's failure says why it couldn't be made or imported",
        arguments: [
            (KeyError.PassphraseRequired, "the key is protected by a passphrase"),
            (KeyError.WrongPassphrase, "the passphrase doesn't open the key"),
            (KeyError.Invalid(reason: "not a key"), "not a key"),
        ]
    )
    internal func keyFailure(error: KeyError, reason: String) {
        #expect(error.reason == reason)
    }
}
