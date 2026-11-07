#if NGTERM_FAKES
    import Foundation
    import NgtermCore
    import Synchronization

    /// A ``PaneClient`` with no native library behind it, for the views' tests.
    ///
    /// It mirrors mobile/android's `FakePane`. ``frame(sinceSeq:)`` answers with what ``frames`` packs (a
    /// ``RowBufferWriter``'s bytes) for the seq asked, and throws ``PaneInputError/Closed`` while it is `nil`;
    /// ``resize(size:)`` and ``setColors(colors:)`` are kept in ``sizes`` and ``colors``, and each seq asked in
    /// ``asked``. As the real terminal wakes (and the core reports a dirty pane) after a resize, new colors or a
    /// selection change, those call ``onWake``. Every method may be called from any thread.
    ///
    /// Input is kept in ``inputs``, in order, and answered as the real terminal would: ``mouse(mouse:)`` reports only
    /// while ``tracking`` (the program asked for the mouse); ``wheel(wheel:)`` answers ``wheelOutcome``;
    /// ``paste(text:allowUnsafe:)`` refuses text with a line break unless allowed (``PaneSendOutcome/unsafe``, as
    /// Ghostty's paste protection does) and writes nothing for empty text; ``selectionText()`` is ``selection``. Input
    /// to a pane with no ``frames`` throws ``PaneInputError/Closed``.
    ///
    /// The chat's actions (``sendPrompt(text:)``, ``interrupt()``, ``answerApproval(approval:label:)``,
    /// ``answerQuestion(question:answers:)``) are kept in ``actions`` and answer ``outcome``, or throw ``actionError``
    /// when set; the next one waits at ``gate`` once taken, while a test holds one on its way.
    /// ``approvalLabels()`` answers ``labels``. A pane with no ``frames`` throws ``PaneInputError/Closed`` for these
    /// too.
    public final class FakePane: PaneClient {
        /// What packs a frame: the bytes for the frame after the seq given.
        public typealias Frames = @Sendable (UInt64) -> Data

        private struct State {
            var frames: Frames?
            var onWake: (@Sendable () -> Void)?
            var tracking = false
            var wheelOutcome = PaneWheelOutcome.scrolled
            var selection: String?
            var sizes: [PaneSize] = []
            var colors: [PaneColors] = []
            var asked: [UInt64] = []
            var inputs: [FakeInput] = []
            var outcome = AgentInputOutcome.sent
            var actionError: PaneInputError?
            var labels: [String] = []
            var gate: FakeGate<Void>?
            var actions: [FakeAction] = []
        }

        private let paneId: String
        private let state: Mutex<State>

        /// What packs its frames; `nil` makes it a pane with no terminal, which refuses frames and input.
        public var frames: Frames? {
            get { state.withLock { $0.frames } }
            set { state.withLock { $0.frames = newValue } }
        }

        /// Runs after a resize, new colors and a selection change: a test points it at the view's dirty mark.
        public var onWake: (@Sendable () -> Void)? {
            get { state.withLock { $0.onWake } }
            set { state.withLock { $0.onWake = newValue } }
        }

        /// Whether the program takes pointer events.
        public var tracking: Bool {
            get { state.withLock { $0.tracking } }
            set { state.withLock { $0.tracking = newValue } }
        }

        /// Where a wheel turn goes.
        public var wheelOutcome: PaneWheelOutcome {
            get { state.withLock { $0.wheelOutcome } }
            set { state.withLock { $0.wheelOutcome = newValue } }
        }

        /// The selection's text as a copy takes it.
        public var selection: String? {
            get { state.withLock { $0.selection } }
            set { state.withLock { $0.selection = newValue } }
        }

        /// The grids it was given, in order.
        public var sizes: [PaneSize] {
            state.withLock { $0.sizes }
        }

        /// The colors it was given, in order.
        public var colors: [PaneColors] {
            state.withLock { $0.colors }
        }

        /// The seqs its frames were asked after, in order.
        public var asked: [UInt64] {
            state.withLock { $0.asked }
        }

        /// The input it took, in order.
        public var inputs: [FakeInput] {
            state.withLock { $0.inputs }
        }

        /// What a chat action answers.
        public var outcome: AgentInputOutcome {
            get { state.withLock { $0.outcome } }
            set { state.withLock { $0.outcome = newValue } }
        }

        /// What a chat action throws instead, when set (a refusal, say).
        public var actionError: PaneInputError? {
            get { state.withLock { $0.actionError } }
            set { state.withLock { $0.actionError = newValue } }
        }

        /// What ``approvalLabels()`` answers.
        public var labels: [String] {
            get { state.withLock { $0.labels } }
            set { state.withLock { $0.labels = newValue } }
        }

        /// A gate the next chat action waits at once taken, until the test opens it (an action on its way).
        public var gate: FakeGate<Void>? {
            get { state.withLock { $0.gate } }
            set { state.withLock { $0.gate = newValue } }
        }

        /// The chat actions taken, in order.
        public var actions: [FakeAction] {
            state.withLock { $0.actions }
        }

        /// A pane named `paneId` whose frames `frames` packs.
        public init(paneId: String = "pane", frames: Frames? = nil) {
            self.paneId = paneId
            state = Mutex(State(frames: frames))
        }

        /// Its id.
        public func id() -> String {
            paneId
        }

        /// The last grid it was given, or an empty one.
        public func size() -> PaneSize {
            state.withLock { $0.sizes.last } ?? PaneSize(cols: 0, rows: 0, cellWidth: 0, cellHeight: 0)
        }

        /// Keeps `size`, and wakes the view.
        public func resize(size: PaneSize) {
            state.withLock { $0.sizes.append(size) }
            wake()
        }

        /// Keeps `colors`, and wakes the view.
        public func setColors(colors: PaneColors) {
            state.withLock { $0.colors.append(colors) }
            wake()
        }

        /// What ``frames`` packs for `sinceSeq`.
        public func frame(sinceSeq: UInt64) throws(PaneInputError) -> Data {
            let packer = state.withLock { state in
                state.asked.append(sinceSeq)
                return state.frames
            }
            guard let packer else {
                throw PaneInputError.Closed
            }
            return packer(sinceSeq)
        }

        /// Takes the key: sent.
        public func key(key: PaneKey) throws(PaneInputError) -> Bool {
            try take(.key(key))
            return true
        }

        /// Takes the text: sent unless empty.
        public func typeText(text: String) throws(PaneInputError) -> Bool {
            try take(.typed(text))
            return !text.isEmpty
        }

        /// Takes the pointer event: reported while the program tracks the mouse.
        public func mouse(mouse: PaneMouse) throws(PaneInputError) -> Bool {
            try take(.mouse(mouse))
            return tracking
        }

        /// Whether the program tracks the mouse: ``tracking``.
        public func mouseTracking() -> Bool {
            tracking
        }

        /// Takes the wheel turn: it goes where ``wheelOutcome`` says.
        public func wheel(wheel: PaneWheel) throws(PaneInputError) -> PaneWheelOutcome {
            try take(.wheel(wheel))
            return wheelOutcome
        }

        /// Takes the paste: empty text writes nothing, and a line break is refused unless allowed.
        public func paste(text: String, allowUnsafe: Bool) throws(PaneInputError) -> PaneSendOutcome {
            try take(.paste(text: text, allowUnsafe: allowUnsafe))
            if text.isEmpty {
                return .empty
            }
            let breaks = text.unicodeScalars.contains { $0 == "\n" || $0 == "\r" }
            return !allowUnsafe && breaks ? .unsafe : .written
        }

        /// Takes the focus change: no program asked to hear of it.
        public func focus(focused: Bool) throws(PaneInputError) -> Bool {
            try take(.focus(focused))
            return false
        }

        /// Takes the selection, and wakes the view.
        public func select(start: CellPoint, end: CellPoint, rectangle: Bool) throws(PaneInputError) {
            try selected(.select(start: start, end: end, rectangle: rectangle))
        }

        /// Takes the selection, and wakes the view: a word was there.
        public func selectWord(at point: CellPoint) throws(PaneInputError) -> Bool {
            try selected(.selectWord(point))
            return true
        }

        /// Takes the selection, and wakes the view: a line was there.
        public func selectLine(at point: CellPoint) throws(PaneInputError) -> Bool {
            try selected(.selectLine(point))
            return true
        }

        /// Takes the selection, and wakes the view: the screen was not blank.
        public func selectAll() throws(PaneInputError) -> Bool {
            try selected(.selectAll)
            return true
        }

        /// Takes the selection's end, and wakes the view.
        public func clearSelection() throws(PaneInputError) {
            try selected(.clearSelection)
        }

        /// ``selection``.
        public func selectionText() -> String? {
            selection
        }

        /// Takes the prompt: ``outcome``.
        public func sendPrompt(text: String) async throws -> AgentInputOutcome {
            try await act(.prompt(text))
        }

        /// Takes the Stop: ``outcome``.
        public func interrupt() async throws -> AgentInputOutcome {
            try await act(.interrupt)
        }

        /// Takes the approval's answer: ``outcome``.
        public func answerApproval(approval: ChatApproval, label: String) async throws -> AgentInputOutcome {
            try await act(.approval(approval, label: label))
        }

        /// Takes the question's answers: ``outcome``.
        public func answerQuestion(
            question: ChatQuestion,
            answers: [QuestionAnswer]
        ) async throws -> AgentInputOutcome {
            try await act(.question(question, answers: answers))
        }

        /// ``labels``.
        public func approvalLabels() throws -> [String] {
            let read = state.withLock { state in state.frames == nil ? nil : state.labels }
            guard let read else {
                throw PaneInputError.Closed
            }
            return read
        }

        private func act(_ action: FakeAction) async throws -> AgentInputOutcome {
            let taken: Result<(FakeGate<Void>?, AgentInputOutcome), PaneInputError> = state.withLock { state in
                guard state.frames != nil else {
                    return .failure(.Closed)
                }
                if let error = state.actionError {
                    return .failure(error)
                }
                state.actions.append(action)
                defer { state.gate = nil }
                return .success((state.gate, state.outcome))
            }
            let (held, answer) = try taken.get()
            await held?.wait()
            return answer
        }

        private func take(_ input: FakeInput) throws(PaneInputError) {
            let open = state.withLock { state in
                guard state.frames != nil else {
                    return false
                }
                state.inputs.append(input)
                return true
            }
            guard open else {
                throw PaneInputError.Closed
            }
        }

        private func selected(_ input: FakeInput) throws(PaneInputError) {
            try take(input)
            wake()
        }

        private func wake() {
            state.withLock { $0.onWake }?()
        }

        deinit {
            // nothing to release
        }
    }
#endif
