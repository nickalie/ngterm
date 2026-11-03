package com.nickalie.ngterm.android.ui.machines

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.CopyButton
import com.nickalie.ngterm.android.ui.components.KeyPlate
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape

/**
 * [machine], reached at [address], asks the user to sign in at [url] before it lets the connect in ([message] is what
 * it said, Tailscale SSH's `check` say). Open link, or a tap on the link in the message, hands it to the browser; Copy
 * link puts it on the clipboard. The sheet stays, waiting, and goes once the connect ends either way. Don't connect,
 * swiping the sheet away or back cancels the connect ([onCancel]).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SignInSheet(
    machine: String,
    address: String,
    message: String,
    url: String,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val uris = LocalUriHandler.current
    ModalBottomSheet(
        onDismissRequest = onCancel,
        modifier = modifier,
        sheetState = sheet,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        SignInQuestion(machine, address, message, url, onOpen = { open(uris, url) }, onCancel = onCancel)
    }
}

// a device with nothing to open the link with leaves Copy link as the way
private fun open(
    uris: UriHandler,
    url: String,
) {
    try {
        uris.openUri(url)
    } catch (_: IllegalArgumentException) {
        // no browser: the link stays on the sheet to copy
    }
}

/** [message] with [url] in it as a link that calls [onOpen]; a message that doesn't show the link gets it under it. */
@Composable
private fun linked(
    message: String,
    url: String,
    onOpen: () -> Unit,
): AnnotatedString {
    val styles =
        TextLinkStyles(SpanStyle(color = MaterialTheme.colorScheme.primary, textDecoration = TextDecoration.Underline))
    val shown = if (url in message) message else listOf(message, url).filter { it.isNotBlank() }.joinToString("\n")
    val start = shown.indexOf(url)
    return buildAnnotatedString {
        append(shown.substring(0, start))
        withLink(LinkAnnotation.Url(url, styles) { onOpen() }) { append(url) }
        append(shown.substring(start + url.length))
    }
}

/** The sign-in request's content, apart from its sheet. */
@Composable
internal fun SignInQuestion(
    machine: String,
    address: String,
    message: String,
    url: String,
    onOpen: () -> Unit,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .navigationBarsPadding()
                .padding(horizontal = spacing.xl)
                .padding(bottom = spacing.lg),
        verticalArrangement = Arrangement.spacedBy(spacing.lg),
    ) {
        StatusGlyph(StatusShape.Diamond, NgtermTheme.status.attention, size = 20.dp)
        Text(
            text = stringResource(R.string.signin_title, machine),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = address,
            style = NgtermTheme.mono.small,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = stringResource(R.string.signin_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        KeyPlate(
            text = linked(message, url, onOpen),
            label = stringResource(R.string.signin_message, machine),
            style = NgtermTheme.mono.small,
        ) { CopyButton(url, label = stringResource(R.string.signin_copy)) }
        Waiting()
        Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Button(onClick = onOpen, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.signin_open))
            }
            TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.signin_cancel))
            }
        }
    }
}

// the connect is alive and goes on by itself: the sheet says so while it waits
@Composable
private fun Waiting() {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
    ) {
        CircularProgressIndicator(modifier = Modifier.size(12.dp), strokeWidth = 2.dp)
        Text(
            text = stringResource(R.string.signin_waiting),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
