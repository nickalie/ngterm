package com.nickalie.ngterm.android.ui.updates

import android.text.format.Formatter
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.onboarding.Problem
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.android.update.UpdateFailure
import com.nickalie.ngterm.android.update.UpdateOffer
import com.nickalie.ngterm.android.update.UpdateState
import com.nickalie.ngterm.android.update.VerifyProblem

/**
 * Where updating stands, on its plate: the state's glyph and headline in the status tones (up to date is healthy,
 * an update needs you, a download is on its way, a failure is a failure), the step from the [installed] version to
 * the offered one, what happens next, a download's progress, and the buttons that move it on. Without its [plate]
 * it lies on whatever holds it (the update sheet).
 */
@Composable
fun UpdatePanel(
    installed: String,
    state: UpdateState,
    actions: UpdateActions,
    modifier: Modifier = Modifier,
    plate: Boolean = true,
) {
    val spacing = NgtermTheme.spacing
    val ground =
        if (plate) {
            Modifier
                .clip(MaterialTheme.shapes.medium)
                .background(MaterialTheme.colorScheme.surfaceContainerLow)
                .padding(start = spacing.lg, end = spacing.sm, top = spacing.md, bottom = spacing.xs)
        } else {
            Modifier
        }
    Column(
        modifier
            .fillMaxWidth()
            .then(ground)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(spacing.xs),
    ) {
        Headline(tone(state), headline(installed, state))
        offer(state)?.let { Step(installed, it.version) }
        Details(installed, state)
        UpdateButtons(state, actions, Modifier.padding(top = spacing.xs))
    }
}

private fun tone(state: UpdateState): StatusTone =
    when (state) {
        UpdateState.Idle -> StatusTone.Idle
        UpdateState.Current -> StatusTone.Online
        is UpdateState.Available -> StatusTone.Attention
        is UpdateState.Failed -> StatusTone.Error
        else -> StatusTone.Connecting
    }

private fun offer(state: UpdateState): UpdateOffer? =
    when (state) {
        is UpdateState.Available -> state.offer
        is UpdateState.Downloading -> state.offer
        is UpdateState.Verifying -> state.offer
        is UpdateState.Installing -> state.offer
        is UpdateState.Failed -> state.offer
        else -> null
    }

@Composable
private fun headline(
    installed: String,
    state: UpdateState,
): String =
    when (state) {
        UpdateState.Idle -> {
            stringResource(R.string.update_version, installed)
        }

        UpdateState.Checking -> {
            stringResource(R.string.update_checking)
        }

        UpdateState.Current -> {
            stringResource(R.string.update_current)
        }

        is UpdateState.Available -> {
            stringResource(R.string.update_available, state.offer.version)
        }

        is UpdateState.Downloading -> {
            stringResource(R.string.update_downloading, state.offer.version)
        }

        is UpdateState.Verifying -> {
            stringResource(R.string.update_verifying)
        }

        is UpdateState.Installing -> {
            stringResource(R.string.update_installing, state.offer.version)
        }

        is UpdateState.Failed -> {
            state.offer?.let { stringResource(R.string.update_failed_title, it.version) }
                ?: stringResource(R.string.update_failed_check_title)
        }
    }

@Composable
private fun Headline(
    tone: StatusTone,
    text: String,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
    ) {
        StatusGlyph(tone.shape, tone.color(NgtermTheme.status))
        Text(
            text,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
    }
}

// the version step in mono: the two versions are read against each other character by character
@Composable
private fun Step(
    installed: String,
    next: String,
) {
    val description = stringResource(R.string.update_step_description, installed, next)
    Text(
        stringResource(R.string.update_step, installed, next),
        style = NgtermTheme.mono.small,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.semantics { contentDescription = description },
    )
}

@Composable
private fun Details(
    installed: String,
    state: UpdateState,
) {
    val context = LocalContext.current
    val size = { bytes: Long -> Formatter.formatShortFileSize(context, bytes) }
    when (state) {
        UpdateState.Idle -> {
            Note(stringResource(R.string.update_idle_note))
        }

        UpdateState.Current -> {
            Note(stringResource(R.string.update_current_note, installed))
        }

        is UpdateState.Available -> {
            Note(stringResource(R.string.update_available_note, size(state.offer.size)))
            if (state.skipped) Note(stringResource(R.string.update_skipped_note))
        }

        is UpdateState.Downloading -> {
            val share = state.received.toFloat() / state.offer.size.coerceAtLeast(1)
            Progress(share)
            Note(stringResource(R.string.update_downloaded, size(state.received), size(state.offer.size)))
        }

        is UpdateState.Verifying -> {
            Progress(null)
        }

        is UpdateState.Installing -> {
            Note(
                stringResource(
                    if (state.waitingForYou) R.string.update_confirm_note else R.string.update_installing_note,
                ),
            )
        }

        is UpdateState.Failed -> {
            Problem(failure(state.failure))
        }

        UpdateState.Checking -> {
            Progress(null)
        }
    }
}

@Composable
private fun failure(failure: UpdateFailure): String =
    when (failure) {
        is UpdateFailure.Check -> {
            stringResource(R.string.update_failed_check, failure.reason)
        }

        is UpdateFailure.Download -> {
            stringResource(R.string.update_failed_download, failure.reason)
        }

        is UpdateFailure.Install -> {
            failure.reason?.let { stringResource(R.string.update_failed_install, it) }
                ?: stringResource(R.string.update_failed_install_plain)
        }

        is UpdateFailure.Verify -> {
            stringResource(
                when (failure.problem) {
                    VerifyProblem.Signature -> R.string.update_failed_signature
                    VerifyProblem.Checksum -> R.string.update_failed_checksum
                    VerifyProblem.Package -> R.string.update_failed_package
                    VerifyProblem.Signer -> R.string.update_failed_signer
                },
            )
        }
    }

@Composable
private fun Note(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

// brass, the tone of what is on its way; no share while the length is unknown (checking, verifying)
@Composable
private fun Progress(share: Float?) {
    val color = NgtermTheme.status.connecting
    val track = MaterialTheme.colorScheme.surfaceContainerHighest
    val modifier = Modifier.fillMaxWidth().padding(end = NgtermTheme.spacing.sm, top = NgtermTheme.spacing.xs)
    if (share == null) {
        LinearProgressIndicator(modifier = modifier, color = color, trackColor = track)
    } else {
        LinearProgressIndicator(progress = { share }, modifier = modifier, color = color, trackColor = track)
    }
}
