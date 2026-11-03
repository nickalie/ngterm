package com.nickalie.ngterm.android.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.runtime.Immutable
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** The 4dp grid every gap and padding comes from. */
@Immutable
data class Spacing(
    val xxs: Dp = 2.dp,
    val xs: Dp = 4.dp,
    val sm: Dp = 8.dp,
    val md: Dp = 12.dp,
    val lg: Dp = 16.dp,
    val xl: Dp = 24.dp,
    val xxl: Dp = 32.dp,
    val xxxl: Dp = 48.dp,
    /** The smallest side of anything tappable. */
    val touchTarget: Dp = 48.dp,
    /** The bus bar's thickness: the copper bar that marks where you are. */
    val busBar: Dp = 4.dp,
    /** The widest a column of cards or rows grows before the space is left empty. */
    val readableWidth: Dp = 640.dp,
)

/**
 * The radii, by rank rather than one for everything: badges and chips are nearly square (4-6dp), cards and rows 12dp,
 * sheets and dialogs 20dp, and only a status capsule is fully round.
 */
val NgtermShapes =
    Shapes(
        extraSmall = RoundedCornerShape(4.dp),
        small = RoundedCornerShape(6.dp),
        medium = RoundedCornerShape(12.dp),
        large = RoundedCornerShape(20.dp),
        extraLarge = RoundedCornerShape(28.dp),
    )
