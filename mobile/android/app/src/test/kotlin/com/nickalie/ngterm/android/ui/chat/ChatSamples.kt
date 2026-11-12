package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.nickalie.ngterm.android.rankCommands
import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.AgentCommandKind
import com.nickalie.ngterm.core.ffi.ChatApproval
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatNoticeKind
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers

/** Chats to draw: a Claude Code conversation about a fix to an install guide, and the cards it waits on. */
object ChatSamples {
    val conversation = ChatBody.Conversation("claude", "c1", "~/docs-site", "claude-opus-4")

    /** The edit's diff. */
    val diff =
        """
        diff --git a/docs/install.md b/docs/install.md
        --- a/docs/install.md
        +++ b/docs/install.md
        @@ -12,7 +12,8 @@ Install ngterm
         Put ngtermctl on the PATH of ssh logins.
        -Check it with `which ngtermctl` in bash.
        +Check it with `command -v ngtermctl`, which works in
        +bash, zsh and fish alike.

         ## Updating
        """.trimIndent()

    private fun tool(
        name: String,
        kind: ChatToolKind,
        title: String,
        status: ChatToolStatus = ChatToolStatus.OK,
        output: String? = null,
    ): ChatBody = ChatBody.Tool(ChatTool(name, kind, title, status, output, null, false))

    /** A call with no result yet. */
    fun running(
        name: String,
        kind: ChatToolKind,
        title: String,
    ) = ChatTool(name, kind, title, ChatToolStatus.RUNNING, null, null, false)

    val prompt =
        event(
            "u1",
            user(
                "The install guide says to run `which ngtermctl`, which fails on fish. " +
                    "Fix the guide and the installer.",
            ),
        )
    val opening = event("a1", reply("I'll look at the installer's PATH check first, then the guide."))
    val grep =
        event(
            "t1",
            tool(
                "Bash",
                ChatToolKind.SHELL,
                "grep -n PATH scripts/install.sh",
                output = "14:if ! which ngtermctl >/dev/null; then\n15:  echo \"ngtermctl is not on the PATH\"",
            ),
        )
    val read = event("t2", tool("Read", ChatToolKind.READ, "scripts/install.sh"))
    val edit =
        event(
            "t3",
            ChatBody.Tool(ChatTool("Edit", ChatToolKind.EDIT, "docs/install.md", ChatToolStatus.OK, null, diff, false)),
        )

    /** A conversation long enough to scroll: prompts, replies, tools of every kind, a subagent and notices. */
    val long: List<ChatEvent> =
        listOf(
            prompt,
            event("th1", ChatBody.Thinking("")),
            opening,
            grep,
            read,
            edit,
            event("task", tool("Task", ChatToolKind.TASK, "Look for which in the other guides")),
            event("s1", tool("Grep", ChatToolKind.SEARCH, "which ngtermctl docs/"), parent = "task"),
            event("s2", reply("Only docs/install.md uses it."), parent = "task"),
            event("t4", tool("Bash", ChatToolKind.SHELL, "npm test", ChatToolStatus.ERROR, output = "1 failing")),
            event("n1", ChatBody.Notice(ChatNoticeKind.INTERRUPT, "Interrupted by the user", null)),
            event("u2", user("Keep going: that test fails on main too.")),
            event("th2", ChatBody.Thinking("**Checking the installer on fish**")),
            event(
                "a2",
                reply(
                    "Done. The guide and the installer now both use\n\n```sh\ncommand -v ngtermctl\n```\n\n" +
                        "which works in **bash**, **zsh** and **fish**.\n\n" +
                        "- `scripts/install.sh`: the check\n- `docs/install.md`: the step",
                ),
            ),
            event("t5", tool("Bash", ChatToolKind.SHELL, "npm run build", ChatToolStatus.RUNNING)),
        )

    /** A permission prompt with the dialog's three options. */
    val approval =
        ApprovalCard(
            id = "ap1",
            approval =
                ChatApproval(
                    "Bash",
                    "rm -rf build && npm run build",
                    "rm -rf build && npm run build",
                    "toolu_9",
                    "/home/user/project",
                    emptyList(),
                ),
            labels =
                listOf(
                    "Yes",
                    "Yes, and don't ask again for rm commands in this project",
                    "No, and tell Claude what to do",
                ),
        )

    /** A multi-select `AskUserQuestion`. */
    val shells =
        ChatQuestion(
            listOf(
                ChatQuestionItem(
                    "Which shells should the guide cover?",
                    "Shells",
                    true,
                    listOf(
                        ChatQuestionOption("bash", "The default on most servers"),
                        ChatQuestionOption("zsh", "macOS's login shell"),
                        ChatQuestionOption("fish", "Needs its own syntax"),
                    ),
                ),
            ),
            ChatToolStatus.RUNNING,
            null,
        )

    /** Claude Code's commands and skills, as an origin lists them. */
    val commands =
        listOf(
            AgentCommand("add-dir", "Add a new working directory", AgentCommandKind.BUILTIN, "<path>"),
            AgentCommand("clear", "Clear the conversation history and free up context", AgentCommandKind.BUILTIN, null),
            AgentCommand(
                "compact",
                "Clear history but keep a summary in context, with optional instructions for it",
                AgentCommandKind.BUILTIN,
                "[instructions]",
            ),
            AgentCommand("context", "Show how the context window is used", AgentCommandKind.BUILTIN, null),
            AgentCommand("cost", "Show the session's cost and duration", AgentCommandKind.BUILTIN, null),
            AgentCommand(
                "docs:release-notes",
                "Draft the release notes from the merged PRs",
                AgentCommandKind.COMMAND,
                null,
            ),
            AgentCommand(
                "frontend-design:frontend-design",
                "Design a distinctive interface",
                AgentCommandKind.PLUGIN,
                null,
            ),
            AgentCommand("review", "Review the current branch against main", AgentCommandKind.SKILL, null),
        )

    /** A composer holding [text], its catalog [commands] (none: still on its way), ranked by the tests' ranking. */
    fun composer(
        text: String,
        commands: List<AgentCommand>? = this.commands,
    ): ChatComposer {
        val gate = CompletableDeferred<List<AgentCommand>>()
        commands?.let(gate::complete)
        val composer = ChatComposer(CoroutineScope(Dispatchers.Unconfined), { gate.await() }, ::rankCommands)
        composer.edit(TextFieldValue(text, TextRange(text.length)))
        return composer
    }

    /** A chat of [events], live, in [conversation]. */
    fun state(
        events: List<ChatEvent>,
        older: OlderPages = OlderPages.AtStart,
    ) = ChatUiState(
        log = ChatLog().append(events),
        phase = ChatPhase.Live,
        conversation = conversation,
        older = older,
    )
}
