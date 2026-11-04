package com.nickalie.ngterm.android.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedIconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef

/**
 * The chat's foot: the prompt field ([composer]'s, growing to [MAX_LINES] lines, then scrolling inside), Stop while
 * the agent works on a turn ([ChatUiState.working]), and Send. While a card waits ([ChatUiState.awaitsCard]) a line
 * over the field says to answer it first, and Send and Stop hold, as their keys would land in its dialog; the field
 * still takes a draft. While an action is on its way ([ChatUiState.busy]) the field holds still and Send shows a
 * spinner. The field takes the input focus through [focusRequester].
 *
 * A hardware keyboard: Ctrl+Enter sends; while the `/` menu lists commands, Up and Down move its highlight and Enter
 * or Tab picks it; Escape (or Back) closes the menu. Back reaches only the [selected] pane's menu: a split's other
 * pane may keep its own open, and Back must not close that one instead.
 */
@Composable
fun Composer(
    composer: ChatComposer,
    state: ChatUiState,
    actions: ChatActions,
    focusRequester: FocusRequester,
    modifier: Modifier = Modifier,
    selected: Boolean = true,
) {
    val spacing = NgtermTheme.spacing
    BackHandler(enabled = selected && composer.menu != null) { composer.dismissMenu() }
    val canSend = !state.busy && !state.awaitsCard && composer.draft.text.isNotBlank()
    Surface(modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.surfaceContainer) {
        // the field and its buttons centered in a reading width, as the rows above are
        Box(contentAlignment = Alignment.TopCenter) {
            Column(
                modifier =
                    Modifier
                        .widthIn(max = NgtermTheme.spacing.readableWidth)
                        .fillMaxWidth()
                        .padding(horizontal = spacing.sm, vertical = spacing.sm),
                verticalArrangement = Arrangement.spacedBy(spacing.xs),
            ) {
                if (state.awaitsCard) HeldLine()
                Row(
                    verticalAlignment = Alignment.Bottom,
                    horizontalArrangement = Arrangement.spacedBy(spacing.xs),
                ) {
                    OutlinedTextField(
                        value = composer.draft,
                        onValueChange = composer::edit,
                        modifier =
                            Modifier
                                .weight(1f)
                                .focusRequester(focusRequester)
                                .onPreviewKeyEvent { composerKey(it, composer, canSend, actions.onSend) },
                        placeholder = { Text(stringResource(R.string.chat_prompt_hint), maxLines = 1) },
                        readOnly = state.busy,
                        maxLines = MAX_LINES,
                        shape = MaterialTheme.shapes.medium,
                        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                    )
                    if (state.working && !state.awaitsCard) {
                        StopButton(enabled = !state.busy, onClick = actions.onStop)
                    }
                    SendButton(enabled = canSend, sending = state.busy && !state.awaitsCard, onClick = actions.onSend)
                }
            }
        }
    }
}

// the most lines the field grows to before it scrolls inside: a long prompt leaves the conversation in sight
private const val MAX_LINES = 6

// why Send and Stop hold: a card waits, and their keys would land in its dialog
@Composable
private fun HeldLine(modifier: Modifier = Modifier) {
    Text(
        text = stringResource(R.string.chat_composer_held),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier =
            modifier
                .padding(horizontal = NgtermTheme.spacing.xs)
                .semantics { liveRegion = LiveRegionMode.Polite },
    )
}

@Composable
private fun SendButton(
    enabled: Boolean,
    sending: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(if (sending) R.string.chat_sending else R.string.chat_send)
    FilledIconButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.padding(bottom = 4.dp).semantics { contentDescription = label },
    ) {
        if (sending) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        } else {
            Icon(painterResource(R.drawable.ic_send), contentDescription = null)
        }
    }
}

@Composable
private fun StopButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(R.string.chat_stop)
    OutlinedIconButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.padding(bottom = 4.dp).semantics { contentDescription = label },
    ) {
        Icon(painterResource(R.drawable.ic_tool_stop), contentDescription = null)
    }
}

// a hardware key in the field: the menu's keys while it lists commands, Escape while it is open, Ctrl+Enter to send
private fun composerKey(
    event: KeyEvent,
    composer: ChatComposer,
    canSend: Boolean,
    onSend: () -> Unit,
): Boolean {
    if (event.type != KeyEventType.KeyDown) return false
    val enter = event.key == Key.Enter || event.key == Key.NumPadEnter
    return when {
        event.key == Key.DirectionDown -> composer.moveHighlight(1)
        event.key == Key.DirectionUp -> composer.moveHighlight(-1)
        (enter && !event.isCtrlPressed) || event.key == Key.Tab -> composer.pickHighlighted()
        event.key == Key.Escape -> composer.dismissMenu()
        enter && event.isCtrlPressed -> canSend.also { if (it) onSend() }
        else -> false
    }
}

/**
 * Keeps [pane]'s draft (its [composer]'s, written in [conversation]) while the app is put away and its process let go,
 * and hands it to [onRestore] (the conversation it was written for, its text) when the app comes back to the same
 * pane; the chat puts it back only into that conversation and only into an empty field, so another pane or
 * conversation starts empty. A draft longer than [KEPT_DRAFT_MAX] isn't kept, as saved state has a size limit.
 */
@Composable
fun KeepDraft(
    pane: PaneRef,
    composer: ChatComposer,
    conversation: String?,
    onRestore: (conversation: String, text: String) -> Unit,
) {
    val named by rememberUpdatedState(conversation)
    val restore by rememberUpdatedState(onRestore)
    val kept = rememberSaveable(pane, saver = keptDraftSaver(pane, composer) { named }) { KeptDraft(null, "") }
    LaunchedEffect(kept) {
        val written = kept.conversation ?: return@LaunchedEffect
        restore(written, kept.text)
    }
}

/** The most characters of a draft [KeepDraft] keeps. */
const val KEPT_DRAFT_MAX = 16_384

/** A draft as [KeepDraft] keeps it: the [conversation] it was written for (`null`: nothing kept) and its [text]. */
internal class KeptDraft(
    val conversation: String?,
    val text: String,
)

/**
 * How [KeepDraft] saves: [pane], the [conversation] and the [composer]'s text, read as the state is saved (nothing for
 * a blank or oversized draft, or before the conversation is named); one saved for another pane is let go.
 */
internal fun keptDraftSaver(
    pane: PaneRef,
    composer: ChatComposer,
    conversation: () -> String?,
): Saver<KeptDraft, Any> =
    Saver(
        save = {
            val text = composer.draft.text
            val named = conversation()
            if (named == null || text.isBlank() || text.length > KEPT_DRAFT_MAX) {
                null
            } else {
                arrayListOf(pane.machineId, pane.sessionId, pane.paneId, named, text)
            }
        },
        restore = { saved ->
            val fields = (saved as? List<*>)?.filterIsInstance<String>()?.takeIf { it.size == KEPT_FIELDS }
            val from = fields?.let { PaneRef(it[0], it[1], it[2]) }
            if (fields != null && from == pane) KeptDraft(fields[KEPT_CONVERSATION], fields[KEPT_TEXT]) else null
        },
    )

// a kept draft's fields: the pane's three ids, the conversation, the text
private const val KEPT_FIELDS = 5
private const val KEPT_CONVERSATION = 3
private const val KEPT_TEXT = 4
