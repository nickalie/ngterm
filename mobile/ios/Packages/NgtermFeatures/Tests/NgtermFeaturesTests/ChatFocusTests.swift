import NgtermCoreFakes
import NgtermDesign
import NgtermTerminal
import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// The chat's prompt field and the keyboard (mobile/android's `ChatPane` focus): a chat the selection moves to takes
/// the keyboard when it was in use, and tells the keyboard its field has it; one the selection leaves lets it go.
@MainActor
@Suite("the chat's focus")
internal struct ChatFocusTests {
    // the key the keyboard knows the chat's prompt field by
    private static let prompt = AnyHashable([AnyHashable(ChatWorld.pane), AnyHashable("prompt")])
    // how many layout passes the screen gets to settle
    private static let passes = 4

    // a chat over a fake core that is never connected, with a conversation in it
    private static func chat() -> ChatModel {
        let core = FakeMobileCore()
        core.connected = ["m1"]
        let sources = ChatModel.Sources(
            core: core,
            handle: { nil },
            isWorking: { false },
            connects: { 0 },
            onEnded: { _ in
                // nothing ends here
            }
        )
        let chat = ChatModel(pane: ChatWorld.pane, sources: sources) { _, items in items }
        chat.take([
            ChatSamples.event("conv", ChatSamples.conversation),
            ChatSamples.event("u1", ChatSamples.user("Fix the install guide.")),
            ChatSamples.event("live", .live),
        ])
        return chat
    }

    private static func screen(_ chat: ChatModel, keyboard: TerminalKeyboard, isSelected: Bool) -> some View {
        ChatScreen(chat: chat, machine: "build-01", keyboard: keyboard, isSelected: isSelected) {
            // the terminal isn't shown here
        }
        .ngtermTheme()
    }

    @Test("a chat selected while the keyboard was in use takes it, and one the selection leaves lets it go")
    internal func handover() async throws {
        let keyboard = TerminalKeyboard()
        // another chat's field had the keyboard when the selection moved
        keyboard.chatField = AnyHashable("another chat's prompt")
        keyboard.handOver()
        let chat = Self.chat()
        let host = UIHostingController(rootView: AnyView(Self.screen(chat, keyboard: keyboard, isSelected: true)))
        // a window of the test's own (UIView's initializer runs UIWindow's, which has no scene here)
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: ScreenVariant.size))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        await Wait.until { keyboard.chatField == Self.prompt }
        #expect(!keyboard.takeChatHandover(), "the handover is taken once")

        host.rootView = AnyView(Self.screen(chat, keyboard: keyboard, isSelected: false))
        await Wait.until { keyboard.chatField == nil }
    }

    @Test("a chat selected while the keyboard wasn't in use leaves it down")
    internal func noHandover() async throws {
        let keyboard = TerminalKeyboard()
        let chat = Self.chat()
        let host = UIHostingController(rootView: Self.screen(chat, keyboard: keyboard, isSelected: true))
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: ScreenVariant.size))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        for _ in 0..<Self.passes {
            window.layoutIfNeeded()
            await Task.yield()
        }
        #expect(keyboard.chatField == nil)
    }
}
