package com.nickalie.ngterm.android.ui.components

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A veil over a pane that can't be used as it is: another device is driving it (with Take control as [action]), its
 * link is down and it is reattaching, or its program ended. The pane's last frame stays dimly visible under it, so
 * the user keeps their place. Lay it over the pane in a `Box`, and give the pane [Modifier.underCover] so TalkBack
 * doesn't reach it either.
 *
 * The veil takes every touch that lands on it (a tap, a scroll, a long press), so none reaches the pane; only its
 * [action] answers.
 */
@Composable
fun Cover(
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    @DrawableRes icon: Int = R.drawable.ic_lock,
    action: SectionAction? = null,
) {
    val spacing = NgtermTheme.spacing
    BoxWithConstraints(
        modifier =
            modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.surface.copy(alpha = VEIL_ALPHA))
                .pointerInput(Unit) { swallowEveryPointer() }
                .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        // centered while it fits; in a pane too short for it (a large font scale) it scrolls, so the action stays
        // reachable
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .heightIn(min = maxHeight)
                    .padding(spacing.xl),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(spacing.sm, Alignment.CenterVertically),
        ) {
            Icon(
                painterResource(icon),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(28.dp),
            )
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Center,
            )
            Text(
                text = body,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = BODY_WIDTH.dp),
            )
            if (action != null) {
                FilledTonalButton(onClick = action.onClick, modifier = Modifier.padding(top = spacing.sm)) {
                    Text(action.label)
                }
            }
        }
    }
}

/**
 * A pane while a [Cover] veils it ([covered]): hidden from TalkBack, which would otherwise still find its nodes under
 * the veil. A modifier rather than a slot, so covering and uncovering the pane keeps it where it is in the
 * composition, with its state.
 */
fun Modifier.underCover(covered: Boolean): Modifier = if (covered) clearAndSetSemantics {} else this

// the veil's own children (its action) see each event first, in the main pass; whatever reaches the veil stops there
private suspend fun PointerInputScope.swallowEveryPointer() {
    awaitPointerEventScope {
        while (true) {
            awaitPointerEvent().changes.forEach { it.consume() }
        }
    }
}

// opaque enough that the cover's text reads over any pane, clear enough that the pane's layout shows through
private const val VEIL_ALPHA = 0.9f
private const val BODY_WIDTH = 360
