package com.nickalie.ngterm.android.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef

/**
 * The chat's foot, in the messenger's shape: one rounded field on the bar with its buttons inside it (Attach, the
 * paperclip, at its start, and at its end one place that Commands, a `/` that puts one into the empty field and opens
 * the menu, gives up to Send, a copper arrow, once there is something to send, as Telegram's bot chats have it), the
 * prompt ([composer]'s, growing to six lines, then scrolling inside) between them; Stop is the activity line's, over
 * it ([ActivityStrip]). A file attached shows over the pill on its way, with Cancel, or why it
 * stopped ([ChatUiState.upload]); its stored path goes into the prompt once it is there. While a card waits
 * ([ChatUiState.awaitsCard]) a line over the field says to answer it first, and Send holds, as its key would land in
 * its dialog; the field still takes a draft. While an action is on its way ([ChatUiState.busy]) the field
 * holds still and Send shows a spinner. The field takes the input focus through [focusRequester].
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
    val sending = state.busy && !state.awaitsCard
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
                state.upload?.let { UploadLine(it, actions.onCancelUpload, actions.onUploadShown) }
                PromptPill(
                    composer = composer,
                    state = state,
                    actions = actions,
                    focusRequester = focusRequester,
                    canSend = canSend,
                    sendShown = composer.draft.text.isNotBlank() || sending,
                    sending = sending,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

// why Send holds: a card waits, and its key would land in its dialog
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
