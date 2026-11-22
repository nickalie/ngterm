import Testing
import UserNotifications

@testable import NgtermPlatform

/// A notice's kind rides with it to the system, so a switch turned off finds the notices of its kind that are up, an
/// earlier run's too.
@MainActor
@Suite("the notices' kinds")
internal struct NoticeKindsTests {
    private static func request(_ id: String, kind: NoticeKind?) -> UNNotificationRequest {
        let notice = Notice(
            id: id,
            threadId: "session:m1:s1",
            title: "t",
            body: "b",
            info: ["machine": "m1"],
            kind: kind
        )
        return UNNotificationRequest(identifier: id, content: SystemNotices.content(of: notice), trigger: nil)
    }

    @Test("the notices of the kinds turned off are found by their kind, one with no kind never")
    internal func foundByKind() {
        let requests = [
            Self.request("ask:m1:s1:A1", kind: .questions),
            Self.request("status:m1:s1", kind: .waiting),
            Self.request("notice:m1:s1:1", kind: .notices),
            Self.request("plain", kind: nil),
        ]
        let found = SystemNotices.ids(of: requests, ofKinds: [.questions, .notices])
        #expect(found == ["ask:m1:s1:A1", "notice:m1:s1:1"])
        #expect(SystemNotices.ids(of: requests, ofKinds: Set(NoticeKind.allCases)).count == 3)
        #expect(SystemNotices.ids(of: requests, ofKinds: [.done]).isEmpty)
    }

    @Test("a notice says what it says, its info carried as given")
    internal func content() {
        let content = Self.request("status:m1:s1", kind: .done).content
        #expect(content.title == "t")
        #expect(content.body == "b")
        #expect(content.threadIdentifier == "session:m1:s1")
        #expect(content.userInfo["machine"] as? String == "m1")
    }
}
