package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.Cover
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.components.swallowEveryPointer
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * The cover over a pane that can't be used as it is ([cover]), with the action that fixes it: **Take control** for a
 * follower ([onTakeLead]), Detach for an exited program ([onDetach]), Retry for a pane that could not be shown
 * ([onRetry]). Reconnecting is not a veil: a light scrim takes the touches and a strip says so, and the last screen
 * stays readable under it. [machine] names where the pane lives.
 */
@Composable
fun LeadCover(
    cover: PaneCover,
    machine: String,
    onTakeLead: () -> Unit,
    onDetach: () -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    when (cover) {
        PaneCover.Follower -> {
            Cover(
                title = stringResource(R.string.session_follower_title),
                body = stringResource(R.string.session_follower_body),
                modifier = modifier,
                action = SectionAction(stringResource(R.string.session_take_control), onTakeLead),
            )
        }

        PaneCover.Attaching -> {
            Cover(
                title = stringResource(R.string.session_attaching_title),
                body = stringResource(R.string.session_attaching_body, machine),
                modifier = modifier,
                icon = null,
            )
        }

        PaneCover.Reconnecting -> {
            Reconnecting(machine, modifier)
        }

        is PaneCover.Exited -> {
            Cover(
                title = stringResource(R.string.session_exited_title),
                body = exitedBody(cover),
                modifier = modifier,
                icon = R.drawable.ic_status_capsule,
                action = SectionAction(stringResource(R.string.machine_sessions_detach), onDetach),
            )
        }

        is PaneCover.Failed -> {
            Cover(
                title = stringResource(R.string.session_pane_failed_title),
                body = cover.reason,
                modifier = modifier,
                icon = R.drawable.ic_alert,
                action = SectionAction(stringResource(R.string.action_retry), onRetry),
            )
        }
    }
}

@Composable
private fun exitedBody(cover: PaneCover.Exited): String =
    when {
        cover.signal != null -> stringResource(R.string.session_exited_signal, cover.signal)
        cover.code != null -> stringResource(R.string.session_exited_code, cover.code.toInt())
        else -> stringResource(R.string.session_exited_unknown)
    }

// the last screen under a light scrim that takes every touch, and a strip at the top saying the link is coming back
@Composable
private fun Reconnecting(
    machine: String,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Box(modifier.fillMaxSize()) {
        Box(
            Modifier
                .matchParentSize()
                .background(MaterialTheme.colorScheme.surface.copy(alpha = SCRIM_ALPHA))
                .pointerInput(Unit) { swallowEveryPointer() },
        )
        Surface(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(spacing.sm)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            shape = MaterialTheme.shapes.small,
        ) {
            Row(
                modifier = Modifier.padding(horizontal = spacing.lg, vertical = spacing.md),
                horizontalArrangement = Arrangement.spacedBy(spacing.md),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                Column(verticalArrangement = Arrangement.spacedBy(spacing.xxs)) {
                    Text(
                        text = stringResource(R.string.session_reconnecting_title, machine),
                        style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    Text(
                        text = stringResource(R.string.session_reconnecting_body),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

// light enough that the last screen reads through it, heavy enough to say it isn't live
private const val SCRIM_ALPHA = 0.4f
