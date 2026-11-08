package com.nickalie.ngterm.android.ui.theme

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.TargetBasedAnimation
import androidx.compose.animation.core.VectorConverter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Theme motion reaches its target at the documented time, with a fast start and a gentle landing. */
class MotionTest {
    @Test
    fun defaultMotionUsesTheDesignDurationsAndSettlesAtTheTarget() {
        val motion = Motion()
        listOf(motion.quick<Float>() to 120L, motion.standard<Float>() to 220L, motion.emphasized<Float>() to 320L)
            .forEach { (spec, duration) ->
                val animation = TargetBasedAnimation(spec, Float.VectorConverter, 0f, 1f)
                assertEquals(duration * NANOS_PER_MILLI, animation.durationNanos)
                assertEquals(0f, animation.getValueFromNanos(0), 0f)
                assertTrue(animation.getValueFromNanos(duration * NANOS_PER_MILLI / 2) > 0.5f)
                assertEquals(1f, animation.getValueFromNanos(animation.durationNanos), 0f)
            }
    }

    @Test
    fun customMotionUsesItsOwnTimingAndEasingForEveryKindOfChange() {
        val motion = Motion(40, 80, 160, FastOutSlowInEasing)
        listOf(motion.quick<Float>() to 40L, motion.standard<Float>() to 80L, motion.emphasized<Float>() to 160L)
            .forEach { (spec, duration) ->
                val animation = TargetBasedAnimation(spec, Float.VectorConverter, 0f, 1f)
                assertEquals(duration * NANOS_PER_MILLI, animation.durationNanos)
                assertEquals(
                    FastOutSlowInEasing.transform(0.5f),
                    animation.getValueFromNanos(animation.durationNanos / 2),
                    0.001f,
                )
            }
    }

    private companion object {
        const val NANOS_PER_MILLI = 1_000_000L
    }
}
