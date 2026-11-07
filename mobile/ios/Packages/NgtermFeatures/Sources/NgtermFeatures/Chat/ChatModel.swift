import Foundation
import NgtermCore
import Observation

/// A shown pane's chat (mobile/android's `ChatViewModel`), from the moment ``PaneChats`` opens it until it closes.
///
/// The events the core streams fold into ``snapshot``, older pages come in on ``loadOlder()``, and the actions
/// (``send()``, ``stop()``, ``answerApproval(id:label:)``, ``answerQuestion(id:answers:)``) go through the pane's
/// handle, each claiming the lead first as the core does; reading never claims it. The prompt being written is the
/// ``composer``'s, and a file it attaches goes to the machine on ``attach(name:size:read:)``.
///
/// - An approval card comes up with the `approval` event and goes with its `approval_done` (or the next `approval`), or
///   when this device answered it; a dialog the core can't confirm as the card's leaves it up, its banner offering the
///   terminal. A question card comes up with a question still running and goes once it is answered, here, or when the
///   core finds its picker no longer on screen. A tap answers the card it was drawn on: one another card replaced
///   meanwhile sends nothing.
/// - A `reset` drops what was shown and starts the new conversation under a divider.
@preconcurrency
@MainActor
@Observable
public final class ChatModel {
    /// What a chat model reads besides its pane's events: the core, the pane's handle, whether the agent works on a
    /// turn, how many times the machine's link came back, and what to do once the chat ended.
    public struct Sources {
        /// The core.
        public let core: any MobileCoreClient
        /// The pane's handle while it is shown.
        public let handle: @MainActor () -> (any PaneClient)?
        /// Whether the session's agent status says the pane's agent works on a turn.
        public let isWorking: @MainActor () -> Bool
        /// How many times the pane's machine's link has come up.
        public let connects: @MainActor () -> Int
        /// The chat (the one given) ended, or never opened.
        public let onEnded: @MainActor (ChatModel) -> Void

        /// The sources given.
        @preconcurrency
        public init(
            core: any MobileCoreClient,
            handle: @escaping @MainActor () -> (any PaneClient)?,
            isWorking: @escaping @MainActor () -> Bool,
            connects: @escaping @MainActor () -> Int,
            onEnded: @escaping @MainActor (ChatModel) -> Void
        ) {
            self.core = core
            self.handle = handle
            self.isWorking = isWorking
            self.connects = connects
            self.onEnded = onEnded
        }
    }

    /// The pane the chat is of.
    public let pane: PaneRef
    /// The chat as the screen shows it.
    public internal(set) var snapshot = ChatSnapshot()
    /// The prompt being written and its `/` menu.
    @ObservationIgnored public let composer: ChatComposer
    @ObservationIgnored private let sources: Sources
    @ObservationIgnored private var tasks: [Task<Void, Never>] = []
    // a connect count the open waits to see passed (it found the link down), else nil
    @ObservationIgnored private var waitingPast: Int?
    @ObservationIgnored private var isClosed = false
    @ObservationIgnored private var uploads: ChatUploads?

    /// Whether the agent works on a turn: the origin's word.
    public var isWorking: Bool {
        sources.isWorking()
    }

    /// Whether a turn runs that Stop can end: the agent works, and the turn was not interrupted.
    public var isRunning: Bool {
        isWorking && !snapshot.isInterrupted
    }

    /// The chat of `pane`, over `sources`, its `/` menu ranking the agent's commands with `filter`.
    @preconcurrency
    public init(
        pane: PaneRef,
        sources: Sources,
        filter: @escaping @MainActor (String, [AgentCommand]) -> [AgentCommand] = { query, items in
            filterCommands(query: query, items: items)
        }
    ) {
        self.pane = pane
        self.sources = sources
        let core = sources.core
        composer = ChatComposer(
            fetch: {
                try await core.agentCommands(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
            },
            filter: filter
        )
    }

    /// Opens the pane's chat at the core.
    ///
    /// While the machine's link is down it is reconnecting and opens once the link is back (``linkCameBack()``); one
    /// that can't be opened otherwise (the origin offers no chat) ends, so the pane shows its terminal.
    internal func open() {
        run { [weak self] in
            await self?.opening()
        }
    }

    /// Closes the chat at the core and stops its work.
    internal func close() {
        isClosed = true
        for task in tasks {
            task.cancel()
        }
        tasks = []
        uploads?.stop()
        // a core that has stopped has no chat open
        try? sources.core.closeChat(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
    }

    /// The pane's machine is connected again: a chat that found it down opens now.
    internal func linkCameBack() {
        guard let past = waitingPast, sources.connects() > past else {
            return
        }
        waitingPast = nil
        snapshot.phase = .loading
        open()
    }

    /// Takes a batch of the chat's events, in order.
    internal func take(_ events: [ChatEvent]) {
        let before = snapshot.approval?.id
        snapshot = snapshot.taking(events)
        if let card = snapshot.approval, card.id != before {
            readLabels(card.id)
        }
        if events.contains(where: { $0.body == .reset }) {
            composer.newConversation()
        }
    }

    /// Takes what became of the chat's link.
    internal func take(_ state: ChatState) {
        switch state {
        case .reconnecting:
            snapshot.phase = .reconnecting

        case .ended:
            ended()
        }
    }

    /// Asks for the page before the oldest row, once at a time, until the start is shown.
    public func loadOlder() {
        let asked = snapshot
        guard asked.older != .loading, asked.older != .atStart, !asked.log.isEmpty else {
            return
        }
        snapshot.older = .loading
        run { [weak self, pane, core = sources.core] in
            let page: [ChatEvent]
            do {
                page = try await core.loadOlder(
                    machineId: pane.machineId,
                    sessionId: pane.sessionId,
                    paneId: pane.paneId
                )
            } catch MobileError.Cancelled {
                // the conversation switched meanwhile: its reset already set the pages afresh
                return
            } catch let error as MobileError {
                self?.pageFailed(error.reason, epoch: asked.epoch)
                return
            } catch {
                return
            }
            self?.pageCame(page, epoch: asked.epoch)
        }
    }

    /// Sends the composer's text as a prompt, its ends trimmed, once at a time and not while a card waits; the field
    /// lets it go once the agent has it, and keeps it when it was not sent.
    public func send() {
        let text = composer.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !snapshot.awaitsCard else {
            return
        }
        let field = composer
        act(card: nil, onSent: { field.sent(text) }, call: { pane in try await pane.sendPrompt(text: text) })
    }

    /// Stop: interrupts the agent's turn (Escape), not while a card waits, as Escape would answer its dialog, nor once
    /// the turn was interrupted.
    public func stop() {
        guard !snapshot.awaitsCard, !snapshot.isInterrupted else {
            return
        }
        act(card: nil) { try await $0.interrupt() }
    }

    /// Answers the approval card `id` (the one its buttons were drawn for) with its button `label`.
    ///
    /// Only while it is still the card up, and only the dialog that asks about its call. The card stays up when the
    /// core can't confirm the dialog is its own.
    public func answerApproval(id: String, label: String) {
        guard let card = snapshot.approval, card.id == id else {
            replaced()
            return
        }
        act(card: card.id) { try await $0.answerApproval(approval: card.approval, label: label) }
    }

    /// Answers the question card `id` (the one its answers were picked on) with `answers`, one per question.
    public func answerQuestion(id: String, answers: [QuestionAnswer]) {
        guard let card = snapshot.question, card.id == id else {
            replaced()
            return
        }
        act(card: card.id, goneIfNotOnScreen: true) { pane in
            try await pane.answerQuestion(question: card.question, answers: answers)
        }
    }

    /// Puts back a draft (`text`) kept from before, once the chat names its conversation: only into the
    /// `conversation` it was written for, and only into an empty field.
    internal func restoreDraft(conversation: String, text: String) {
        run { [weak self] in
            let named = Observations { @MainActor [weak self] in self?.snapshot.conversation?.id }
            for await now in named where now != nil {
                if now == conversation {
                    self?.composer.restore(text)
                }
                return
            }
        }
    }

    /// Brings the list to its newest row, the card at its foot under it, the next time it is on screen.
    internal func showEnd() {
        snapshot.toEnd = true
    }

    /// The list has gone to its newest row.
    public func endShown() {
        snapshot.toEnd = false
    }

    /// The problem has been shown and dismissed.
    public func problemShown() {
        snapshot.problem = nil
    }

    /// Sends the file `name` of `size` bytes (`nil`: unknown) that `read` gives to the pane's machine, unless one is on
    /// its way, and puts its stored path into the prompt; Cancel stops it while bytes are still to go.
    @preconcurrency
    public func attach(name: String, size: Int?, read: @escaping @Sendable () async throws -> Data) {
        uploader().attach(name: name, size: size, read: read)
    }

    /// Stops the upload on its way, unless every byte has gone: too late then, the machine stores it.
    public func cancelUpload() {
        uploads?.cancel()
    }

    /// The upload's failure was seen: it goes.
    public func uploadShown() {
        if case .failed = snapshot.upload {
            snapshot.upload = nil
        }
    }

    // the chat's uploads, made as the first is attached
    private func uploader() -> ChatUploads {
        if let uploads {
            return uploads
        }
        let made = ChatUploads(machineId: pane.machineId, core: sources.core) { [weak self] upload in
            self?.snapshot.upload = upload
        } current: { [weak self] in
            self?.snapshot.upload
        } stored: { [weak self] path in
            self?.composer.insertPath(ChatComposer.promptPath(path))
        }
        uploads = made
        return made
    }

    // runs `work` as the chat's own, cancelled when it closes
    private func run(_ work: @escaping @MainActor () async -> Void) {
        guard !isClosed else {
            return
        }
        tasks.removeAll(where: \.isCancelled)
        tasks.append(Task { await work() })
    }

    private func opening() async {
        // the connects counted before asking: one after the core found the link down brings it back
        let seen = sources.connects()
        do {
            try await sources.core.openChat(machineId: pane.machineId, sessionId: pane.sessionId, paneId: pane.paneId)
        } catch MobileError.NotConnected {
            snapshot.phase = .reconnecting
            waitingPast = seen
            linkCameBack()
        } catch is MobileError {
            ended()
        } catch {
            // the chat closed meanwhile
        }
    }

    private func ended() {
        snapshot.phase = .ended
        snapshot.isBusy = false
        sources.onEnded(self)
    }

    private func pageFailed(_ reason: String, epoch: Int) {
        if snapshot.epoch == epoch {
            snapshot.older = .failed(reason: reason)
        }
    }

    private func pageCame(_ page: [ChatEvent], epoch: Int) {
        if snapshot.epoch == epoch {
            snapshot = snapshot.withPage(page)
        }
    }

    // a tap on a card another one replaced before it landed: nothing is sent
    private func replaced() {
        if !snapshot.isBusy {
            snapshot.problem = .notOnScreen
        }
    }

    // one action at a time; `card` is the card it answers, which goes once answered, or, `goneIfNotOnScreen`, once
    // the core finds its prompt gone
    private func act(
        card: String?,
        goneIfNotOnScreen: Bool = false,
        onSent: @escaping @MainActor () -> Void = {
            // nothing more to do once it is sent
        },
        call: @escaping @Sendable (any PaneClient) async throws -> AgentInputOutcome
    ) {
        guard !snapshot.isBusy else {
            return
        }
        guard let target = sources.handle() else {
            snapshot.problem = .closed
            return
        }
        snapshot.isBusy = true
        snapshot.problem = nil
        run { [weak self] in
            let problem: ChatProblem?
            do {
                problem = ChatProblem(try await call(target))
            } catch let error as PaneInputError {
                problem = ChatProblem(error)
            } catch {
                problem = .closed
            }
            self?.acted(card: card, problem: problem, goneIfNotOnScreen: goneIfNotOnScreen, onSent: onSent)
        }
    }

    private func acted(card: String?, problem: ChatProblem?, goneIfNotOnScreen: Bool, onSent: () -> Void) {
        if problem == nil {
            onSent()
        }
        let isGone = problem == nil || (goneIfNotOnScreen && problem == .notOnScreen)
        if let card, isGone {
            snapshot = snapshot.without(card: card)
        }
        snapshot.isBusy = false
        snapshot.problem = problem
    }

    // the dialog's labels as the screen shows them now, for the card still up
    private func readLabels(_ id: String) {
        guard let target = sources.handle() else {
            return
        }
        run { [weak self] in
            guard let labels = try? await target.approvalLabels(), let self, var card = snapshot.approval, card.id == id
            else {
                return
            }
            card.labels = labels
            snapshot.approval = card
        }
    }

    isolated deinit {
        for task in tasks {
            task.cancel()
        }
    }
}
