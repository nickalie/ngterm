package com.nickalie.ngterm.android.ui.asks

import org.junit.Assert.assertEquals
import org.junit.Test

/** The grant restarts the attached sessions' service, whose notification was dropped before it. */
class NotificationPermissionTest {
    private var restarts = 0

    private fun answer(
        allowed: Boolean,
        attached: Map<String, Set<String>>,
    ) = afterPermissionAnswer(allowed, attached) { restarts += 1 }

    @Test
    fun aGrantWithASessionAttachedRestartsTheService() {
        answer(allowed = true, attached = mapOf("m1" to setOf("s1")))
        assertEquals(1, restarts)
    }

    @Test
    fun aRefusalOrNothingAttachedStartsNothing() {
        answer(allowed = false, attached = mapOf("m1" to setOf("s1")))
        answer(allowed = true, attached = emptyMap())
        answer(allowed = true, attached = mapOf("m1" to emptySet()))
        assertEquals(0, restarts)
    }
}
