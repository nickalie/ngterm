package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.nickalie.ngterm.android.ui.asks.hasForbiddenControls
import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** The agent's slash commands and skills as the `/` menu has them, for the conversation on screen. */
sealed interface CommandCatalog {
    /** Not asked for in this conversation yet: the menu asks as it opens. */
    data object Unasked : CommandCatalog

    /** On its way from the origin. */
    data object Fetching : CommandCatalog

    /** The origin gave none (it predates the catalog, or the link went): the menu asks again as it next opens. */
    data object Failed : CommandCatalog

    /** What the origin listed, A to Z as it came. */
    data class Ready(
        val commands: List<AgentCommand>,
    ) : CommandCatalog
}

/** The `/` menu over the composer, for what follows the `/` ([query]). */
sealed interface SlashMenu {
    val query: String

    /**
     * No commands to list: the catalog is on its way ([fetching]), or the origin listed none (it predates the catalog,
     * the link went, or the agent has none). Either is a quiet row, never an error: the prompt still goes as typed.
     */
    data class Waiting(
        override val query: String,
        val fetching: Boolean,
    ) : SlashMenu

    /** The [commands] matching [query], best first (none: nothing matches); Enter or Tab picks the [highlighted]. */
    data class Commands(
        override val query: String,
        val commands: List<AgentCommand>,
        val highlighted: Int,
    ) : SlashMenu
}

/**
 * A chat's composer: the prompt being written ([field], the field's text, selection and the IME's composition alike)
 * and the `/` menu over it. A `/` at the start of a text with no space or line break in it opens the menu ([menu]) on
 * the agent's commands, narrowed by what follows the `/` through [filter] (the core's `filter_commands`, the
 * palettes' ranking) as it is typed, the IME's composition included; a space, a line break or deleting the `/` closes
 * it, and so does Escape until the text changes. Picking a command ([pick]) puts `/name ` in the field (a Codex
 * skill's `$name ` as it stands), the cursor at its end.
 *
 * The catalog comes once per conversation ([fetch], asked as the menu first opens, and again after a failure when it
 * next opens, or at once after a new conversation while it is open). Main thread only: the field must take each edit
 * in the frame it was made, or the IME loses its place.
 */
@Stable
class ChatComposer(
    private val scope: CoroutineScope,
    private val fetch: suspend () -> List<AgentCommand>,
    private val filter: (String, List<AgentCommand>) -> List<AgentCommand>,
) {
    /** What the field holds. */
    var draft: TextFieldValue by mutableStateOf(TextFieldValue())
        private set

    /** The catalog for the conversation on screen. */
    var catalog: CommandCatalog by mutableStateOf(CommandCatalog.Unasked)
        private set

    // the text whose menu Escape closed: it stays closed until the text changes
    private var dismissed: String? by mutableStateOf(null)
    private var highlight by mutableIntStateOf(0)

    // the catalog asked for last: an answer to an earlier ask (a conversation since) is dropped
    private var asking: Job? = null
    private var asks = 0

    // the last ranking, kept while neither the query nor the catalog changes (a cursor move or a frame redraws)
    private var ranked: Triple<String, List<AgentCommand>, List<AgentCommand>>? = null

    /** What follows the `/` while the text opens the menu (Escape aside), else `null`. */
    val query: String? get() = slashQuery(draft.text)

    /** The menu as it shows now, or `null` while it is closed. */
    val menu: SlashMenu?
        get() {
            val query = query?.takeIf { draft.text != dismissed } ?: return null
            return when (val catalog = catalog) {
                is CommandCatalog.Ready -> {
                    if (catalog.commands.isEmpty()) return SlashMenu.Waiting(query, fetching = false)
                    val commands = rank(query, catalog.commands)
                    SlashMenu.Commands(query, commands, highlight.coerceIn(0, (commands.size - 1).coerceAtLeast(0)))
                }

                CommandCatalog.Fetching -> {
                    SlashMenu.Waiting(query, fetching = true)
                }

                CommandCatalog.Unasked, CommandCatalog.Failed -> {
                    SlashMenu.Waiting(query, fetching = false)
                }
            }
        }

    /**
     * The field takes [value] as typed: line breaks as the clipboard brings them (`\r\n`, `\r`) become `\n`, and an
     * edit that would add another control character is refused whole. The menu opens or narrows with it.
     */
    fun edit(value: TextFieldValue) {
        val next = value.withPlainBreaks()
        if (hasForbiddenControls(next.text, multiline = true)) return
        val opened = query == null && slashQuery(next.text) != null
        if (next.text != draft.text) {
            highlight = 0
            dismissed = null
        }
        draft = next
        if (query != null) ask(retry = opened)
    }

    /** Puts [command] in the field in place of the `/` and the query, then a space, the cursor after it. */
    fun pick(command: AgentCommand) {
        val text = insertion(command) + " "
        draft = TextFieldValue(text, TextRange(text.length))
        highlight = 0
    }

    /** Picks the highlighted command; `false` when the menu lists none (closed, waiting, or nothing matches). */
    fun pickHighlighted(): Boolean {
        val menu = menu as? SlashMenu.Commands
        val command = menu?.commands?.getOrNull(menu.highlighted)
        command?.let(::pick)
        return command != null
    }

    /** Moves the highlight [by] rows, staying on the list; `false` when the menu lists none. */
    fun moveHighlight(by: Int): Boolean {
        val menu = (menu as? SlashMenu.Commands)?.takeIf { it.commands.isNotEmpty() } ?: return false
        highlight = (menu.highlighted + by).coerceIn(0, menu.commands.lastIndex)
        return true
    }

    /** Closes the menu (Escape, Back) until the text changes; `false` when it was closed. */
    fun dismissMenu(): Boolean {
        if (menu == null) return false
        dismissed = draft.text
        return true
    }

    /** [text] went to the agent: the field lets it go, unless it holds something else by now. */
    internal fun sent(text: String) {
        if (draft.text.trim() == text) draft = TextFieldValue()
    }

    /** Puts a [text] kept from before back in the field, unless the field holds something already. */
    internal fun restore(text: String) {
        if (draft.text.isEmpty()) edit(TextFieldValue(text, TextRange(text.length)))
    }

    /** A new conversation: its catalog is asked for afresh, at once while the menu is open. */
    internal fun newConversation() {
        asking?.cancel()
        asks++
        catalog = CommandCatalog.Unasked
        ranked = null
        if (query != null) ask(retry = false)
    }

    // asks for the catalog when there is none for this conversation; [retry] asks again after a failure too
    private fun ask(retry: Boolean) {
        val due = catalog == CommandCatalog.Unasked || (retry && catalog == CommandCatalog.Failed)
        if (!due) return
        val ask = ++asks
        catalog = CommandCatalog.Fetching
        asking =
            scope.launch {
                val got =
                    try {
                        CommandCatalog.Ready(fetch())
                    } catch (_: MobileException) {
                        // an older origin, a link that went, or the core stopped: no list, and no error either
                        CommandCatalog.Failed
                    }
                if (ask == asks) catalog = got
            }
    }

    private fun rank(
        query: String,
        commands: List<AgentCommand>,
    ): List<AgentCommand> {
        ranked?.let { (q, from, to) -> if (q == query && from === commands) return to }
        return filter(query, commands).also { ranked = Triple(query, commands, it) }
    }
}

/** What follows a leading `/` in [text] while the text is that and one word, else `null`. */
internal fun slashQuery(text: String): String? =
    if (text.startsWith('/') && text.none { it.isWhitespace() }) text.substring(1) else null

/** What picking this command puts in the field, before its space: `/name`, or a Codex skill's `$name` as it stands. */
internal fun insertion(command: AgentCommand): String =
    if (command.name.startsWith('$') || command.name.startsWith('/')) command.name else "/${command.name}"

// line breaks as one `\n` each, the selection and composition kept on the same characters
private fun TextFieldValue.withPlainBreaks(): TextFieldValue {
    if ('\r' !in text) return this
    val plain = text.replace("\r\n", "\n").replace('\r', '\n')

    // an offset in [text] as an offset in [plain]: each `\r\n` before it is one character shorter
    fun at(offset: Int): Int {
        var dropped = 0
        for (index in 0 until (offset - 1).coerceAtLeast(0)) {
            if (text[index] == '\r' && text[index + 1] == '\n') dropped++
        }
        return offset - dropped
    }
    return TextFieldValue(plain, TextRange(at(selection.start), at(selection.end)))
}
