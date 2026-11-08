import Foundation
import NgtermPlatform
import Testing
import UserNotifications

/// The app's notifications through the system's center, in the hosted app (an unhosted test bundle has no center):
/// what a notice shows, and the buttons it registers and takes down with it, in the order the posts and withdrawals
/// were made.
///
/// Each test's actions carry a tag of its own, by which it finds the categories the system reads back. The suite runs
/// one test at a time, as every post sets the app's whole set of categories.
@MainActor
@Suite("the system's notifications", .serialized, .timeLimit(.minutes(1)))
internal struct SystemNoticesTests {
    private let tag = UUID().uuidString

    /// The system's registered category offering the action `id`, if any.
    private static func category(offering id: String) async -> UNNotificationCategory? {
        let categories = await UNUserNotificationCenter.current().notificationCategories()
        return categories.first { category in
            category.actions.contains { $0.identifier == id }
        }
    }

    /// Withdraws the notices and waits for that withdrawal even when the check throws.
    private static func withPosted(_ posted: [Notice], checking: () async throws -> Void) async rethrows {
        let notices = SystemNotices()
        for notice in posted {
            notices.post(notice)
        }
        await notices.delivered()
        let ids = posted.map(\.id)
        do {
            try await checking()
        } catch {
            notices.withdraw(ids)
            await notices.delivered()
            throw error
        }
        notices.withdraw(ids)
        await notices.delivered()
    }

    private func action(_ name: String) -> String {
        "\(name)-\(tag)"
    }

    private func ask(_ name: String, actions: [NoticeAction]) -> Notice {
        Notice(
            id: "\(name)-\(tag)",
            threadId: "m1/s1",
            title: "Claude asks",
            body: "May it run the tests?",
            subtitle: "box",
            info: ["machine": "m1", "session": "s1"],
            actions: actions
        )
    }

    @Test("a notice's actions are registered as its buttons, each asking for the device to be unlocked")
    internal func actionsAreRegistered() async throws {
        let notice = ask(
            "ask",
            actions: [
                NoticeAction(id: action("allow"), title: "Allow"),
                NoticeAction(id: action("deny"), title: "Deny", isDestructive: true),
                NoticeAction(id: action("reply"), title: "Reply", replyPlaceholder: "Your answer"),
            ]
        )
        try await Self.withPosted([notice]) {
            let category = try #require(await Self.category(offering: action("allow")))
            #expect(category.actions.map(\.identifier) == [action("allow"), action("deny"), action("reply")])
            #expect(category.actions.map(\.title) == ["Allow", "Deny", "Reply"])
            #expect(
                category.actions.map(\.options) == [
                    [.authenticationRequired],
                    [.authenticationRequired, .destructive],
                    [.authenticationRequired],
                ])
            #expect(!(category.actions[0] is UNTextInputNotificationAction))
            let reply = try #require(category.actions.last as? UNTextInputNotificationAction)
            #expect(reply.textInputPlaceholder == "Your answer")
            #expect(reply.textInputButtonTitle == "Send")
        }
    }

    @Test("a posted notice reaches the user as made: its words, thread, buttons and what its tap carries")
    internal func presentedAsPosted() async throws {
        let presented = PresentedNotices()
        defer { presented.restore() }
        let asking = ask("shown", actions: [NoticeAction(id: action("shown"), title: "Allow")])
        let plain = ask("plain", actions: [])
        let askingPresentation = Task.immediate { try await presented.presented(asking.id) }
        try await Self.withPosted([asking, plain]) {
            let shown = try await askingPresentation.value
            let category = try #require(await Self.category(offering: action("shown")))
            let expected = PresentedNotices.Shown(
                title: "Claude asks",
                subtitle: "box",
                body: "May it run the tests?",
                threadId: "m1/s1",
                category: category.identifier,
                info: ["machine": "m1", "session": "s1"]
            )
            #expect(shown == expected)
            #expect(try await presented.presented(plain.id).category.isEmpty)
        }
    }

    @Test("a throwing check still withdraws its notices and buttons before returning")
    internal func throwingCheckWithdraws() async {
        let notice = ask("throws", actions: [NoticeAction(id: action("throws"), title: "Allow")])
        await #expect(throws: CancellationError.self) {
            try await Self.withPosted([notice]) {
                #expect(await Self.category(offering: action("throws")) != nil)
                throw CancellationError()
            }
        }
        #expect(await Self.category(offering: action("throws")) == nil)
        let center = UNUserNotificationCenter.current()
        #expect(await !center.deliveredNotifications().contains { $0.request.identifier == notice.id })
        #expect(await !center.pendingNotificationRequests().contains { $0.identifier == notice.id })
    }

    @Test("a presentation wait ends on cancellation, including cancellation before the wait starts")
    internal func cancelledPresentationWait() async {
        let presented = PresentedNotices()
        defer { presented.restore() }
        let waiting = Task.immediate { try await presented.presented(action("waiting")) }
        waiting.cancel()
        await #expect(throws: CancellationError.self) { try await waiting.value }

        let cancelled = Task { try await presented.presented(action("cancelled")) }
        cancelled.cancel()
        await #expect(throws: CancellationError.self) { try await cancelled.value }
    }

    @Test("a notice that never arrives times out and the app's delegate is restored")
    internal func presentationTimeoutRestoresDelegate() async {
        let center = UNUserNotificationCenter.current()
        let previous = center.delegate
        do {
            let presented = PresentedNotices()
            defer { presented.restore() }
            await #expect(throws: PresentedNotices.TimedOut.self) {
                try await presented.presented(action("missing"), timeout: .zero)
            }
        }
        #expect(center.delegate === previous)
    }

    @Test("withdrawing a notice takes its buttons down, and another notice's stay")
    internal func withdrawTakesItsButtonsDown() async {
        let notices = SystemNotices()
        let first = ask("first", actions: [NoticeAction(id: action("first"), title: "Allow")])
        let second = ask("second", actions: [NoticeAction(id: action("second"), title: "Allow")])
        notices.post(first)
        notices.post(second)
        notices.withdraw([first.id])
        await notices.delivered()
        #expect(await Self.category(offering: action("first")) == nil)
        #expect(await Self.category(offering: action("second")) != nil)
        notices.withdraw([second.id])
        await notices.delivered()
        #expect(await Self.category(offering: action("second")) == nil)
    }

    @Test("a notice withdrawn just after it was posted doesn't come back")
    internal func withdrawnRightAfterThePost() async {
        let notices = SystemNotices()
        let notice = ask("brief", actions: [NoticeAction(id: action("brief"), title: "Allow")])
        notices.post(notice)
        notices.withdraw([notice.id])
        await notices.delivered()
        #expect(await Self.category(offering: action("brief")) == nil)
        let center = UNUserNotificationCenter.current()
        #expect(await !center.deliveredNotifications().contains { $0.request.identifier == notice.id })
        #expect(await !center.pendingNotificationRequests().contains { $0.identifier == notice.id })
    }

    @Test("a notice with no actions registers no buttons, and an empty withdrawal changes nothing")
    internal func plainNoticeAndEmptyWithdrawal() async {
        let notices = SystemNotices()
        let asking = ask("asking", actions: [NoticeAction(id: action("asking"), title: "Allow")])
        notices.post(asking)
        await notices.delivered()
        let before = await UNUserNotificationCenter.current().notificationCategories()
        let plain = ask("plain", actions: [])
        notices.post(plain)
        notices.withdraw([])
        await notices.delivered()
        #expect(await UNUserNotificationCenter.current().notificationCategories() == before)
        notices.withdraw([asking.id, plain.id])
        await notices.delivered()
    }

    @Test("with nothing posted, waiting for delivery returns at once")
    internal func nothingToDeliver() async {
        await SystemNotices().delivered()
    }

    @Test("the permission is the user's answer as the system keeps it")
    internal func permission() async {
        let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
        let expected: NoticePermission =
            switch status {
            case .notDetermined:
                .undecided

            case .denied:
                .denied

            default:
                .allowed
            }
        #expect(await SystemNotices().permission() == expected)
    }
}
