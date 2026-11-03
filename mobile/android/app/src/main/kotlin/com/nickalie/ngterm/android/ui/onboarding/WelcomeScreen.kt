package com.nickalie.ngterm.android.ui.onboarding

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape

/** The width of the first run's column: a line of its body stays under 80 characters. */
internal val OnboardingWidth = 520.dp

/**
 * The first run's first screen: the app has no key yet. Create one (the main action) or import one; [busy] while a
 * key is being made, [failure] when making one failed.
 */
@Composable
fun WelcomeScreen(
    busy: Boolean,
    failure: String?,
    onCreate: () -> Unit,
    onImport: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    ScreenFrame(modifier = modifier, maxWidth = OnboardingWidth) {
        Image(
            painterResource(R.drawable.ic_ngterm_mark),
            contentDescription = null,
            modifier = Modifier.padding(top = spacing.xxl).size(56.dp),
        )
        Text(
            text = stringResource(R.string.welcome_title),
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = stringResource(R.string.welcome_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Icon(
                painterResource(R.drawable.ic_lock),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.secondary,
                modifier = Modifier.size(20.dp),
            )
            Text(
                text = stringResource(R.string.welcome_keystore_note),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (failure != null) {
            Problem(stringResource(R.string.welcome_create_failed, failure))
        }
        Column(
            modifier = Modifier.padding(top = spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            BusyButton(
                label = stringResource(if (busy) R.string.welcome_creating else R.string.welcome_create),
                busy = busy,
                onClick = onCreate,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedButton(onClick = onImport, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.welcome_import))
            }
        }
    }
}

/** The main action of a step, with a spinner in front of its words while it runs (and no second tap). */
@Composable
internal fun BusyButton(
    label: String,
    busy: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Button(onClick = { if (!busy) onClick() }, modifier = modifier) {
        if (busy) {
            CircularProgressIndicator(
                modifier = Modifier.size(18.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.onPrimary,
            )
            Text(label, modifier = Modifier.padding(start = NgtermTheme.spacing.sm))
        } else {
            Text(label)
        }
    }
}

/** What went wrong with a step, in the error color behind the error glyph; TalkBack announces it. */
@Composable
internal fun Problem(
    text: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.Top,
    ) {
        StatusGlyph(
            StatusShape.Triangle,
            NgtermTheme.status.error,
            modifier = Modifier.padding(top = NgtermTheme.spacing.xs),
            size = 14.dp,
        )
        Text(text = text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error)
    }
}
