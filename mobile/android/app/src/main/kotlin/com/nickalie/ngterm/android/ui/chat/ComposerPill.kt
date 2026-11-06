package com.nickalie.ngterm.android.ui.chat

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * The composer's rounded field and the buttons inside it, as a messenger's input bar holds its own: Attach (the
 * paperclip, [AttachButton]) at its start, the prompt, and one slot at its end that Commands and Send share, as
 * Telegram's bot chats share theirs (Stop is the activity strip's, [ActivityStrip]): Commands
 * (a `/` that starts one) while there is nothing to send, Send once there is ([sendShown]), the one turning into the
 * other in place, so the prompt keeps its width as it is typed. Every button in it is a 48dp target, and they stay on
 * the prompt's last line as it grows.
 */
@Composable
internal fun PromptPill(
    composer: ChatComposer,
    state: ChatUiState,
    actions: ChatActions,
    focusRequester: FocusRequester,
    canSend: Boolean,
    sendShown: Boolean,
    sending: Boolean,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.heightIn(min = PillHeight),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = RoundedCornerShape(PillHeight / 2),
    ) {
        Row(verticalAlignment = Alignment.Bottom) {
            AttachButton(enabled = state.upload !is ChatUpload.Sending, onPick = actions.onPick)
            PromptField(composer, state.busy, focusRequester, canSend, actions.onSend, Modifier.weight(1f))
            EndSlot(send = sendShown) { send ->
                if (send) {
                    SendButton(enabled = canSend, sending = sending, onClick = actions.onSend)
                } else {
                    CommandsButton(
                        enabled = !state.busy,
                        onClick = {
                            composer.edit(TextFieldValue("/", TextRange(1)))
                            focusRequester.requestFocus()
                        },
                    )
                }
            }
        }
    }
}

// the pill's last place, Commands' or Send's: the one leaving shrinks and fades as the other grows in from its middle,
// both 48dp, so nothing beside it moves
@Composable
private fun EndSlot(
    send: Boolean,
    content: @Composable (send: Boolean) -> Unit,
) {
    val motion = NgtermTheme.motion
    AnimatedContent(
        targetState = send,
        transitionSpec = {
            (scaleIn(motion.quick(), initialScale = SWAP_SCALE) + fadeIn(motion.quick()))
                .togetherWith(scaleOut(motion.quick(), targetScale = SWAP_SCALE) + fadeOut(motion.quick()))
        },
        contentAlignment = Alignment.Center,
        label = "send or commands",
    ) { content(it) }
}

// how small the leaving button shrinks to, and the arriving one starts from
private const val SWAP_SCALE = 0.6f

// Send, in the pill's last place: the return key's arrow in copper, no plate of its own (the pill is its ground);
// a spinner ("Sending") on its way
@Composable
private fun SendButton(
    enabled: Boolean,
    sending: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(if (sending) R.string.chat_sending else R.string.chat_send)
    IconButton(
        onClick = onClick,
        enabled = enabled,
        colors = IconButtonDefaults.iconButtonColors(contentColor = MaterialTheme.colorScheme.primary),
        modifier = modifier.semantics { contentDescription = label },
    ) {
        if (sending) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        } else {
            Icon(painterResource(R.drawable.ic_send), contentDescription = null)
        }
    }
}

// the prompt itself: no outline of its own (the pill is its edge), the hint while it is empty
@Composable
private fun PromptField(
    composer: ChatComposer,
    busy: Boolean,
    focusRequester: FocusRequester,
    canSend: Boolean,
    onSend: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val style = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface)
    BasicTextField(
        value = composer.draft,
        onValueChange = composer::edit,
        modifier =
            modifier
                .padding(vertical = (PillHeight - FieldLine) / 2)
                .focusRequester(focusRequester)
                .onPreviewKeyEvent { composerKey(it, composer, canSend, onSend) },
        readOnly = busy,
        textStyle = style.copy(lineHeight = FieldLineSp),
        maxLines = MAX_LINES,
        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        decorationBox = { field ->
            Box(contentAlignment = Alignment.CenterStart) {
                if (composer.draft.text.isEmpty()) {
                    Text(
                        stringResource(R.string.chat_prompt_hint),
                        style = style.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                field()
            }
        },
    )
}

// the pill's height on one line (every button in it a 48dp target), and one line of the prompt
private val PillHeight = 48.dp
private val FieldLine = 24.dp
private val FieldLineSp = 24.sp

// the most lines the field grows to before it scrolls inside: a long prompt leaves the conversation in sight
private const val MAX_LINES = 6

// the `/` the agent's commands start with, in the terminal's face, as the button that puts one into the empty field,
// takes the focus and so opens the menu
@Composable
private fun CommandsButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(R.string.chat_commands)
    IconButton(onClick = onClick, enabled = enabled, modifier = modifier.semantics { contentDescription = label }) {
        Text("/", style = ChatType.code.copy(fontSize = 18.sp), color = LocalContentColor.current)
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
