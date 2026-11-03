package com.nickalie.ngterm.android.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * The bus bar: the short copper bar that marks where you are, at the leading edge of the selected destination or
 * session (or the top edge of a bottom-bar item, [vertical] false). It is the design's one loud element, so it marks
 * nothing else. It grows in from its middle when [active] turns on, and shrinks away when it turns off.
 */
@Composable
fun BusBar(
    active: Boolean,
    modifier: Modifier = Modifier,
    vertical: Boolean = true,
    length: Dp = 24.dp,
) {
    val thickness = NgtermTheme.spacing.busBar
    val grown by animateFloatAsState(
        targetValue = if (active) 1f else 0f,
        animationSpec = NgtermTheme.motion.quick(),
        label = "bus bar",
    )
    Box(
        modifier =
            modifier
                .size(width = if (vertical) thickness else length, height = if (vertical) length else thickness)
                .graphicsLayer {
                    if (vertical) scaleY = grown else scaleX = grown
                    alpha = grown
                }.background(MaterialTheme.colorScheme.primary, RoundedCornerShape(thickness / 2)),
    )
}
