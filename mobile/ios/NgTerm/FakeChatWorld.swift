#if DEBUG
    import Foundation
    import NgtermCore
    import NgtermCoreFakes
    import Synchronization

    /// The fake session's first pane running Claude Code, for the chat's UI tests.
    ///
    /// Its conversation is `c1` in `~/docs-site`, whose catch-up holds a prompt and its reply, and its agent answers
    /// the chat's actions as Claude Code 2.1.322 does through the origin:
    ///
    /// - a prompt shows as the user's row, then (after a moment on its way, as a real one takes, or 8 s for one that
    ///   ends "take your time") the reply "Done: …";
    /// - "ask me" asks ``shells`` the way Claude's `AskUserQuestion` comes: a permission prompt carrying the questions,
    ///   its transcript writing the call only once it is answered. Answered, the call's row comes with the prompt's end
    ///   and "Going with <pick>.";
    /// - "run the build" asks to allow `npm run build` (the dialog's Yes and No); allowed, the command's row comes with
    ///   the prompt's end and "The build passed.";
    /// - "ask me, then drop" asks as "ask me" does, then the chat's link drops (on the cue ``FakeCues/Cue/chatDrop``)
    ///   and comes back (on ``FakeCues/Cue/chatBack``) with the question answered in the terminal meanwhile: the
    ///   catch-up writes the answered call and the core ends the prompt. Cues, not a clock, as a UI test looks at the
    ///   card and the drop before they end, however slow the simulator answers it;
    /// - "write a lot" replies with 30 paragraphs, "Paragraph 30 ends here." last.
    internal final class FakeChatWorld: Sendable {
        /// How long the agent takes: a prompt on its way, and one that ends "take your time".
        internal struct Pace: Sendable {
            /// As a real agent takes, near enough.
            internal static let real = Self(
                typing: .milliseconds(typingMilliseconds),
                slowTyping: .seconds(slowTypingSeconds)
            )
            private static let typingMilliseconds = 600
            private static let slowTypingSeconds = 8

            internal let typing: Duration
            internal let slowTyping: Duration
        }

        /// The pane the agent runs in.
        internal static let pane = PaneRef(
            machineId: FakeSessionWorld.machineId,
            sessionId: FakeSessionWorld.sessionId,
            paneId: "P1"
        )
        /// The question "ask me" asks.
        internal static let shells = [shell]
        private static let shell = ChatQuestionItem(
            question: "Which shell should the guide use?",
            header: "Shell",
            multiSelect: false,
            options: [
                ChatQuestionOption(label: "bash", description: "The default on most servers"),
                ChatQuestionOption(label: "fish", description: nil),
            ]
        )
        private static let paragraphs = 30

        /// The origin's word that the pane runs Claude Code in `c1`.
        internal static var agentEvent: MobileEvent {
            .paneAgent(
                machineId: pane.machineId,
                sessionId: pane.sessionId,
                paneId: pane.paneId,
                agent: "claude",
                conversationId: "c1"
            )
        }

        /// What the pane's chat says as it opens: the conversation so far, then that it follows.
        internal static var catchUp: [ChatEvent] {
            [
                event("conv", .conversation(agent: "claude", conversationId: "c1", cwd: "~/docs-site", model: nil)),
                event("u0", .user(text: "Write the install guide", images: 0)),
                event("a0", .assistant(text: "The guide is in docs/install.md.")),
                event("live", .live),
            ]
        }

        private let core: FakeMobileCore
        private let pace: Pace
        private let cue: @Sendable (FakeCues.Cue) async -> Void
        private let turns = Mutex(0)
        private let prompts = Mutex<[String: ChatApproval]>([:])

        /// The agent, telling the app through `core`, at `pace`, each step a UI test looks at first waiting for `cue`.
        internal init(core: FakeMobileCore, pace: Pace, cue: @escaping @Sendable (FakeCues.Cue) async -> Void) {
            self.core = core
            self.pace = pace
            self.cue = cue
        }

        private static func event(_ id: String, _ body: ChatBody) -> ChatEvent {
            ChatEvent(id: id, cursor: 0, parent: nil, body: body)
        }

        // Claude's AskUserQuestion as its PermissionRequest hook carries it
        private static func asking(_ questions: [ChatQuestionItem]) -> ChatApproval {
            ChatApproval(
                tool: "AskUserQuestion",
                title: questions.first?.question ?? "",
                detail: nil,
                toolUseId: nil,
                cwd: "~/docs-site",
                questions: questions
            )
        }

        private static func allowing(_ tool: String, _ title: String) -> ChatApproval {
            ChatApproval(tool: tool, title: title, detail: title, toolUseId: nil, cwd: "~/docs-site", questions: [])
        }

        private static func question(_ status: ChatToolStatus, output: String?) -> ChatBody {
            .question(question: ChatQuestion(questions: shells, status: status, output: output))
        }

        /// The agent's pane: its terminal as `frames` packs it, its dialog's options, and its replies to the chat.
        internal func pane(frames: @escaping FakePane.Frames) -> FakePane {
            let agentPane = FakePane(paneId: Self.pane.paneId, frames: frames)
            agentPane.labels = ["Yes", "No"]
            agentPane.onAction = { [self] action in
                await reply(to: action)
            }
            return agentPane
        }

        private func reply(to action: FakeAction) async {
            switch action {
            case .prompt(let text):
                await prompted(text)

            case .question(_, let answers):
                let pick = answers.first?.picked.first ?? answers.first?.other ?? ""
                answered("question") { turn in
                    [
                        Self.event("toolu_q\(turn)", Self.question(.ok, output: pick)),
                        Self.event("reply-q\(turn)", .assistant(text: "Going with \(pick).")),
                    ]
                }

            case .approval(_, let label) where label == "Yes":
                answered("build") { turn in
                    [
                        Self.event(
                            "toolu_b\(turn)",
                            .tool(
                                tool: ChatTool(
                                    name: "Bash",
                                    kind: .shell,
                                    title: "npm run build",
                                    status: .ok,
                                    output: "built in 2.1s",
                                    diff: nil,
                                    truncated: false
                                )
                            )
                        ),
                        Self.event("reply-b\(turn)", .assistant(text: "The build passed.")),
                    ]
                }

            case .approval, .interrupt:
                break
            }
        }

        private func prompted(_ text: String) async {
            let turn = turns.withLock { turns in
                turns += 1
                return turns
            }
            try? await Task.sleep(for: text.hasSuffix("take your time") ? pace.slowTyping : pace.typing)
            let user = Self.event("u\(turn)", .user(text: text, images: 0))
            switch text {
            case "ask me", "ask me, then drop":
                let id = "question-\(turn)"
                tell([user, ask(id, Self.asking(Self.shells))])
                if text == "ask me, then drop" {
                    Task { [self] in
                        await dropped(answering: id, turn: turn)
                    }
                }

            case "run the build":
                let approval = Self.allowing("Bash", "npm run build")
                tell([user, ask("build-\(turn)", approval)])

            case "write a lot":
                let lines = (1...Self.paragraphs).map { "Paragraph \($0) ends here." }
                tell([user, Self.event("a\(turn)", .assistant(text: lines.joined(separator: "\n\n")))])

            default:
                tell([user, Self.event("a\(turn)", .assistant(text: "Done: \(text)."))])
            }
        }

        // the link drops; the question is answered in the terminal meanwhile, and the catch-up says so
        private func dropped(answering id: String, turn: Int) async {
            await cue(.chatDrop)
            core.emit(.chatState(pane: Self.pane, state: .reconnecting))
            await cue(.chatBack)
            guard let approval = prompts.withLock({ $0.removeValue(forKey: id) }) else {
                return
            }
            tell([
                Self.event("toolu_q\(turn)", Self.question(.ok, output: "bash")),
                Self.event(id, .approvalDone(approval: approval)),
                Self.event("live-\(turn)", .live),
            ])
        }

        private func ask(_ id: String, _ approval: ChatApproval) -> ChatEvent {
            prompts.withLock { $0[id] = approval }
            return Self.event(id, .approval(approval: approval))
        }

        // the prompt waiting whose id starts with `kind` is answered: what the transcript writes, then its end
        private func answered(_ kind: String, rows: (Int) -> [ChatEvent]) {
            let waiting = prompts.withLock { prompts in
                let id = prompts.keys.first { $0.hasPrefix(kind) }
                return id.flatMap { id in prompts.removeValue(forKey: id).map { (id: id, approval: $0) } }
            }
            guard let waiting, let turn = Int(waiting.id.split(separator: "-").last ?? "") else {
                return
            }
            tell(rows(turn) + [Self.event(waiting.id, .approvalDone(approval: waiting.approval))])
        }

        private func tell(_ events: [ChatEvent]) {
            core.emit(.chat(pane: Self.pane, events: events))
        }

        deinit {
            // nothing to release: the core it tells outlives it
        }
    }

#endif
