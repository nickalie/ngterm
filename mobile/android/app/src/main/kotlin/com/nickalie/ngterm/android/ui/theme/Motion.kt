package com.nickalie.ngterm.android.ui.theme

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Immutable

private val EmphasizedDecelerate = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1f)

/**
 * Motion answers an action and shows what changed; nothing moves on its own. The durations are short because the
 * terminal is what the user came for. Compose scales every animation by the system's animator duration scale, so
 * "Remove animations" turns all of these into cuts with no code of ours.
 */
@Immutable
data class Motion(
    /** A state flip in place: a selection, a badge's change. */
    val quickMillis: Int = 120,
    /** Something opening or closing: the rail's labels, a banner. */
    val standardMillis: Int = 220,
    /** A layout swapping wholesale: a destination, a cover. */
    val emphasizedMillis: Int = 320,
    /** Material's emphasized-decelerate: fast out of the touch, settling into place. */
    val easing: Easing = EmphasizedDecelerate,
) {
    /** The tween for a [quickMillis] change. */
    fun <T> quick(): FiniteAnimationSpec<T> = tween(quickMillis, easing = easing)

    /** The tween for a [standardMillis] change. */
    fun <T> standard(): FiniteAnimationSpec<T> = tween(standardMillis, easing = easing)

    /** The tween for an [emphasizedMillis] change. */
    fun <T> emphasized(): FiniteAnimationSpec<T> = tween(emphasizedMillis, easing = easing)
}
