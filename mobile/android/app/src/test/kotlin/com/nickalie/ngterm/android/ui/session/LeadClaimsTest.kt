package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import org.junit.Assert.assertEquals
import org.junit.Test

/** A claim takes the lead once, on its session's first follower report, and holds the cover off until it lands. */
class LeadClaimsTest {
    private val key = SessionKey("m1", "s1")
    private val attached = mapOf("m1" to setOf("s1"))
    private val taken = mutableListOf<PaneRef>()
    private val claims = LeadClaims()

    private fun settle(
        role: PaneRole?,
        on: Map<String, Set<String>> = attached,
    ) = claims.settle(on, mapOf(key to LiveSession(panes = mapOf("P1" to PaneLive(role, covered = true))))) {
        taken += it
    }

    @Test
    fun aFollowerTakesTheLeadOnceAndIsHeldUntilItStopsFollowing() {
        claims.claim(key)
        assertEquals(setOf(key), claims.held.value)
        settle(null)
        assertEquals(setOf(key), claims.held.value)
        settle(PaneRole.FOLLOWER)
        settle(PaneRole.FOLLOWER)
        assertEquals(listOf(PaneRef("m1", "s1", "P1")), taken)
        assertEquals(setOf(key), claims.held.value)
        settle(null)
        assertEquals(emptySet<SessionKey>(), claims.held.value)
        settle(PaneRole.FOLLOWER)
        assertEquals(1, taken.size)
        assertEquals(emptySet<SessionKey>(), claims.held.value)
    }

    @Test
    fun aLeaderFirstDropsTheClaimSoALaterTakeoverIsTheUsers() {
        claims.claim(key)
        settle(PaneRole.LEADER)
        assertEquals(emptySet<SessionKey>(), claims.held.value)
        settle(PaneRole.FOLLOWER)
        assertEquals(emptyList<PaneRef>(), taken)
    }

    @Test
    fun aDetachDropsTheClaimAndATakeoverOnItsWay() {
        claims.claim(key)
        settle(null, on = emptyMap())
        assertEquals(emptySet<SessionKey>(), claims.held.value)
        settle(PaneRole.FOLLOWER)
        assertEquals(emptyList<PaneRef>(), taken)
        claims.claim(key)
        settle(PaneRole.FOLLOWER)
        settle(PaneRole.FOLLOWER, on = emptyMap())
        assertEquals(emptySet<SessionKey>(), claims.held.value)
    }
}
