import NgtermCore

/// Chat events for the tests, as mobile/android's `ChatSamples` and its helpers make them.
internal enum ChatSamples {
    /// A Claude Code conversation in `~/docs-site`.
    internal static let conversation = ChatBody.conversation(
        agent: "claude",
        conversationId: "c1",
        cwd: "~/docs-site",
        model: "claude-opus-4"
    )

    /// An approval of a Bash command.
    internal static let approval = ChatApproval(
        tool: "Bash",
        title: "rm -rf build && npm run build",
        detail: "rm -rf build && npm run build",
        toolUseId: "toolu_9",
        cwd: "/home/user/project",
        questions: []
    )

    /// Claude's `AskUserQuestion` as its permission prompt carries it, before its transcript holds the call.
    internal static let asked = ChatApproval(
        tool: "AskUserQuestion",
        title: "Which shell should the guide use?",
        detail: nil,
        toolUseId: "toolu_q",
        cwd: "/home/user/project",
        questions: shells().questions
    )

    /// The event `id` saying `body`.
    internal static func event(_ id: String, _ body: ChatBody) -> ChatEvent {
        ChatEvent(id: id, cursor: 0, parent: nil, body: body)
    }

    /// The event `id` saying `body`, under the task `parent`.
    internal static func event(_ id: String, _ body: ChatBody, parent: String) -> ChatEvent {
        ChatEvent(id: id, cursor: 0, parent: parent, body: body)
    }

    /// A prompt.
    internal static func user(_ text: String) -> ChatBody {
        .user(text: text, images: 0)
    }

    /// A reply.
    internal static func reply(_ text: String) -> ChatBody {
        .assistant(text: text)
    }

    /// A command named `name` running `title`, with no result yet.
    internal static func tool(_ name: String, _ title: String) -> ChatBody {
        tool(name, title, .running, kind: .shell, output: nil)
    }

    /// A command named `name` running `title`, as `status`, with its `output`.
    internal static func tool(_ name: String, _ title: String, _ status: ChatToolStatus, output: String?) -> ChatBody {
        tool(name, title, status, kind: .shell, output: output)
    }

    /// A tool call named `name` with `title`, as `status`, of `kind`.
    internal static func tool(
        _ name: String,
        _ title: String,
        _ status: ChatToolStatus,
        kind: ChatToolKind
    ) -> ChatBody {
        tool(name, title, status, kind: kind, output: nil)
    }

    /// A tool call named `name` with `title`, as `status`, of `kind`, with its `output`.
    internal static func tool(
        _ name: String,
        _ title: String,
        _ status: ChatToolStatus,
        kind: ChatToolKind,
        output: String?
    ) -> ChatBody {
        .tool(
            tool: ChatTool(
                name: name,
                kind: kind,
                title: title,
                status: status,
                output: output,
                diff: nil,
                truncated: false
            )
        )
    }

    /// An `AskUserQuestion` with one single-select question, waiting.
    internal static func question() -> ChatBody {
        question(.running)
    }

    /// An `AskUserQuestion` with one single-select question, as `status`.
    internal static func question(_ status: ChatToolStatus) -> ChatBody {
        .question(question: shells(status))
    }

    /// One question with two options, waiting.
    internal static func shells() -> ChatQuestion {
        shells(.running)
    }

    /// One question with two options, as `status`.
    internal static func shells(_ status: ChatToolStatus) -> ChatQuestion {
        let options = [
            ChatQuestionOption(label: "bash", description: "The default on most servers"),
            ChatQuestionOption(label: "fish", description: nil),
        ]
        let item = ChatQuestionItem(
            question: "Which shell should the guide use?",
            header: "Shell",
            multiSelect: false,
            options: options
        )
        return ChatQuestion(questions: [item], status: status, output: nil)
    }

    /// A slash command with no description.
    internal static func command(_ name: String) -> AgentCommand {
        command(name, "")
    }

    /// A slash command.
    internal static func command(_ name: String, _ description: String) -> AgentCommand {
        AgentCommand(name: name, description: description, kind: .builtin, argumentHint: nil)
    }
}
