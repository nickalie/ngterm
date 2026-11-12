import NgtermCore

/// What a pane's chat shows (mobile/android's `ChatUiState`): its rows (``log``), where it stands (``phase``), the
/// ``conversation`` its origin named, whether older pages remain (``older``), the row the list keeps in place after a
/// page went in above it (``anchor``), the cards pinned above the composer (``approval``, ``question``), whether an
/// action is on its way (``isBusy``), why the last one did nothing (``problem``, until dismissed), the ``account`` the
/// agent is signed in with (kept through a reset, as the pane's agent is), the file the composer attaches
/// (``upload``), and whether the list is to go to its newest row and follow from there (``toEnd``: a notification's
/// tap opened the chat to the card at its foot), until it has.
public struct ChatSnapshot: Equatable, Sendable {
    /// The rows.
    public internal(set) var log = ChatLog()
    /// Where the chat stands.
    public internal(set) var phase = ChatPhase.loading
    /// The conversation the origin named.
    public internal(set) var conversation: ChatConversation?
    /// Whether older pages remain.
    public internal(set) var older = OlderPages.more
    /// The row a page that went in above keeps in place.
    public internal(set) var anchor: String?
    /// The permission prompt the agent waits on.
    public internal(set) var approval: ApprovalCard?
    /// The question the agent waits on.
    public internal(set) var question: OpenQuestion?
    /// Whether an action is on its way.
    public internal(set) var isBusy = false
    /// Why the last action did nothing.
    public internal(set) var problem: ChatProblem?
    /// The agent's account and weekly limit.
    public internal(set) var account: ChatAccount?
    /// Whether the list is to go to its newest row.
    public internal(set) var toEnd = false
    /// The file the composer attaches.
    public internal(set) var upload: ChatUpload?
    /// Whether the transcript's last turn was interrupted (Stop, or a declined call): no hook says so, so the origin's
    /// status stays working until the next turn ends.
    public internal(set) var isInterrupted = false
    // the questions whose picker was found answered or gone: their card stays down until the event says so
    internal var dismissed: Set<String> = []
    // a question Claude's permission prompt asks before its transcript holds the call (written only once answered)
    internal var prompted: OpenQuestion?
    // bumped by each reset, so a page asked for before it is dropped
    internal var epoch = 0

    /// The rows, oldest first.
    public var items: [ChatItem] {
        log.items
    }

    /// Whether a card waits on an answer, so a prompt or Stop would land in its dialog: the composer holds Send, and
    /// the foot leaves out the activity line with its Stop.
    public var awaitsCard: Bool {
        approval != nil || question != nil
    }

    /// A chat with nothing read yet.
    public init() {
        // every part starts empty
    }

    // whether the conversation's last turn stays interrupted after `event`: an interrupt notice ends it; the agent at
    // work again (a reply, thinking, a call or a question asked) or a prompt typed lifts it, though not a `/command`
    // alone, which may run no turn, nor a call or a question ending (Codex ends the interrupted turn's calls after its
    // notice). A subagent's rows say nothing of the main turn.
    private static func interrupted(_ before: Bool, after event: ChatEvent) -> Bool {
        guard event.parent == nil else {
            return before
        }
        switch event.body {
        case .notice(let kind, _, _):
            return kind == .interrupt || before

        case .user(let text, _):
            return text.drop(while: \.isWhitespace).hasPrefix("/") && before

        case .tool(let tool):
            return tool.status != .running && before

        case .question(let question):
            return question.status != .running && before

        case .assistant, .thinking:
            return false

        default:
            return before
        }
    }

    private static func isRow(_ body: ChatBody) -> Bool {
        switch body {
        case .reset, .live, .conversation, .approval, .approvalDone, .account:
            false

        default:
            true
        }
    }

    private static func asks(_ body: ChatBody, _ questions: [ChatQuestionItem]) -> Bool {
        guard case .question(let question) = body else {
            return false
        }
        return question.questions == questions
    }

    /// The chat with a batch of events taken in, in order: the rows a run of them makes go into the log at once.
    ///
    /// A row written after a permission prompt that asks the same questions takes over its card; an earlier one (the
    /// question asked again) leaves it up.
    public func taking(_ events: [ChatEvent]) -> Self {
        var next = self
        var rows: [ChatEvent] = []
        for event in events {
            if Self.isRow(event.body) {
                rows.append(event)
                next.isInterrupted = Self.interrupted(next.isInterrupted, after: event)
                continue
            }
            next.append(rows)
            rows = []
            next.meta(event)
        }
        next.append(rows)
        return next
    }

    /// The chat with an older page in above the rows, the top row it leaves in place held there: a placeholder whose
    /// task the page brought moves up to the task's place, so the first row the page didn't move is the anchor.
    public func withPage(_ page: [ChatEvent]) -> Self {
        var next = self
        guard !page.isEmpty else {
            next.older = .atStart
            return next
        }
        let paged = log.prepending(page)
        var waiting: Set<String> = []
        var rows: Set<String> = []
        for item in paged.items {
            rows.insert(item.key)
            if case .subagent = item {
                waiting.insert(item.key)
            }
        }
        let shown = items.filter { $0 != .divider }
        let kept = shown.first { item in
            if case .subagent = item {
                return waiting.contains(item.key)
            }
            return true
        }
        let top = kept ?? shown.first { rows.contains($0.key) }
        next.setLog(paged)
        next.older = .more
        next.anchor = top?.key
        return next
    }

    /// The chat with the card `id` answered, here or elsewhere.
    public func without(card id: String) -> Self {
        var next = self
        if next.approval?.id == id {
            next.approval = nil
        }
        if next.question?.id == id {
            next.dismissed.insert(id)
        }
        next.refreshQuestion()
        return next
    }

    private mutating func append(_ rows: [ChatEvent]) {
        guard !rows.isEmpty else {
            return
        }
        if let asked = prompted?.question.questions, rows.contains(where: { Self.asks($0.body, asked) }) {
            prompted = nil
        }
        setLog(log.appending(rows))
    }

    private mutating func setLog(_ next: ChatLog) {
        log = next
        refreshQuestion()
    }

    private mutating func refreshQuestion() {
        question = log.openQuestion(dismissed: dismissed) ?? prompted.flatMap { dismissed.contains($0.id) ? nil : $0 }
    }

    // an event that is no row: the conversation switched or named, the backlog read, a permission prompt up or done,
    // the account read (one that knows nothing clears it)
    private mutating func meta(_ event: ChatEvent) {
        switch event.body {
        case .reset:
            var fresh = Self()
            fresh.log = log.reset()
            fresh.phase = phase
            fresh.isBusy = isBusy
            fresh.problem = problem
            fresh.account = account
            fresh.toEnd = toEnd
            fresh.upload = upload
            fresh.epoch = epoch + 1
            self = fresh

        case .live:
            phase = .live

        case .conversation(let agent, let id, let cwd, let model):
            conversation = ChatConversation(agent: agent, id: id, cwd: cwd, model: model)

        case .approval(let approval) where !approval.questions.isEmpty:
            prompted = OpenQuestion(
                id: event.id,
                question: ChatQuestion(questions: approval.questions, status: .running, output: nil)
            )
            refreshQuestion()

        case .approval(let approval):
            if self.approval?.id != event.id {
                self.approval = ApprovalCard(id: event.id, approval: approval, labels: [])
            }

        case .approvalDone where prompted?.id == event.id:
            prompted = nil
            refreshQuestion()

        case .approvalDone:
            if approval?.id == event.id {
                approval = nil
            }

        case .account(let email, let weekly):
            account = email == nil && weekly == nil ? nil : ChatAccount(email: email, weekly: weekly)

        default:
            break
        }
    }
}
