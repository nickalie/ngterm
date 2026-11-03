package com.nickalie.ngterm.android.ui.components

import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.currentNavLayout
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A screen of reading and filling in (the first run, the machine form, a host key): an optional top row with its
 * [exit], its [title] as a heading and its [action] (Save), then [content] in a column that scrolls, no wider than
 * [maxWidth] and centered in wider windows, with the phone's or the tablet's gutter. It pads for the system bars, the
 * cutout and the keyboard, so a focused field stays above the keyboard.
 */
@Composable
fun ScreenFrame(
    modifier: Modifier = Modifier,
    title: String? = null,
    exit: ScreenExit? = null,
    action: (@Composable () -> Unit)? = null,
    maxWidth: Dp = NgtermTheme.spacing.readableWidth,
    content: @Composable ColumnScope.() -> Unit,
) {
    val spacing = NgtermTheme.spacing
    val gutter = if (currentNavLayout() == NavLayout.Bar) spacing.lg else spacing.xl
    Column(modifier = modifier.fillMaxSize().safeDrawingPadding()) {
        if (title != null || exit != null || action != null) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = spacing.touchTarget + spacing.sm)
                        .padding(horizontal = spacing.xs),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (exit != null) {
                    IconButton(onClick = exit.onClick) {
                        Icon(painterResource(exit.icon), contentDescription = exit.label)
                    }
                }
                Text(
                    text = title.orEmpty(),
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier =
                        Modifier
                            .weight(1f)
                            .padding(horizontal = if (exit == null) gutter - spacing.xs else spacing.xs)
                            .semantics { heading() },
                )
                action?.invoke()
            }
        }
        Box(
            modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),
            contentAlignment = Alignment.TopCenter,
        ) {
            Column(
                modifier =
                    Modifier
                        .widthIn(max = maxWidth)
                        .fillMaxWidth()
                        .padding(horizontal = gutter, vertical = spacing.lg),
                verticalArrangement = Arrangement.spacedBy(spacing.lg),
                content = content,
            )
        }
    }
}

/** A screen's way out: an icon button with its words for TalkBack (Back, Close). */
data class ScreenExit(
    @param:DrawableRes val icon: Int,
    val label: String,
    val onClick: () -> Unit,
)
