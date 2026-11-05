package com.nickalie.ngterm.android.ui.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
 * The composer's rounded field and the buttons inside it: Commands (a `/` that starts one in an empty field, or closes
 * the menu while it is open), the prompt, Attach (the paperclip, [AttachButton]), and Stop while the agent works on a
 * turn ([ChatUiState.running]) and no card waits. Every button in it is a 48dp target, and they stay on the prompt's
 * last line as it grows.
 */
@Composable
internal fun PromptPill(
    composer: ChatComposer,
    state: ChatUiState,
    actions: ChatActions,
    focusRequester: FocusRequester,
    canSend: Boolean,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier.heightIn(min = PillHeight),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = RoundedCornerShape(PillHeight / 2),
    ) {
        Row(verticalAlignment = Alignment.Bottom) {
            CommandsButton(
                enabled = !state.busy && (composer.menu != null || composer.draft.text.isEmpty()),
                onClick = {
                    if (!composer.dismissMenu()) composer.edit(TextFieldValue("/", TextRange(1)))
                    focusRequester.requestFocus()
                },
            )
            PromptField(composer, state.busy, focusRequester, canSend, actions.onSend, Modifier.weight(1f))
            AttachButton(enabled = state.upload !is ChatUpload.Sending, onPick = actions.onPick)
            if (state.running && !state.awaitsCard) {
                StopButton(enabled = !state.busy, onClick = actions.onStop)
            }
        }
    }
}

/** Send's place beside the pill: it grows in from its middle once there is something to send, and out when sent. */
@Composable
internal fun SendSlot(
    visible: Boolean,
    content: @Composable () -> Unit,
) {
    val motion = NgtermTheme.motion
    AnimatedVisibility(
        visible = visible,
        enter = scaleIn(motion.quick()) + fadeIn(motion.quick()),
        exit = scaleOut(motion.quick()) + fadeOut(motion.quick()),
    ) { content() }
}

/** Send: the return key's arrow in a filled copper circle as tall as the pill; a spinner ("Sending") on its way. */
@Composable
internal fun SendButton(
    enabled: Boolean,
    sending: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(if (sending) R.string.chat_sending else R.string.chat_send)
    FilledIconButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.size(PillHeight).semantics { contentDescription = label },
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

// the `/` the agent's commands start with, in the terminal's face, as the button that starts one
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

// inside the pill at its end, while the agent works: the interrupt's square
@Composable
private fun StopButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(R.string.chat_stop)
    IconButton(onClick = onClick, enabled = enabled, modifier = modifier.semantics { contentDescription = label }) {
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
