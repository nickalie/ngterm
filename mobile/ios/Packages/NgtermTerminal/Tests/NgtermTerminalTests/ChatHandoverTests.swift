import NgtermTerminal
import Testing

/// The keyboard's handover from a chat's prompt field: the session view moving the selection hands the keyboard on from
/// a chat as from a terminal, and only once.
@MainActor
@Suite("the keyboard handed on from a chat")
internal struct ChatHandoverTests {
    @Test("a chat's field with the keyboard hands it on once; with no terminal or chat holding it, nothing is handed")
    internal func handover() {
        let keyboard = TerminalKeyboard()
        keyboard.handOver()
        #expect(!keyboard.takeChatHandover(), "nothing had the keyboard")
        keyboard.chatField = "P1"
        keyboard.handOver()
        #expect(keyboard.takeChatHandover())
        #expect(!keyboard.takeChatHandover(), "taken once")
        keyboard.chatField = nil
        keyboard.handOver()
        #expect(!keyboard.takeChatHandover(), "a chat that let its field go hands nothing on")
    }
}
