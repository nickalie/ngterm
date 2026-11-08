package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRole
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The cover reflects the pane's most urgent state, and only reconnecting leaves its last screen readable. */
class PaneCoverTest {
    @Test
    fun aShowFailureWinsAndAMissingTerminalWaitsBeforeAnyRoleOrEnd() {
        val lost = PaneLive(PaneRole.FOLLOWER, covered = true, end = PaneEnd.Lost("network lost"))
        assertEquals(PaneCover.Failed("cannot show pane"), PaneCover.of(lost, false, "cannot show pane"))
        assertEquals(PaneCover.Attaching, PaneCover.of(lost, false, null))
        assertEquals(PaneCover.Attaching, PaneCover.of(null, false, null))
    }

    @Test
    fun anEndedPaneShowsItsExitInsteadOfItsOldRole() {
        val exited = PaneLive(PaneRole.FOLLOWER, covered = true, end = PaneEnd.Exited(7u, "TERM"))
        val cover = PaneCover.of(exited, true, null)
        assertEquals(PaneCover.Exited(7u, "TERM"), cover)
        assertTrue(cover.veils)
        val lost = PaneCover.of(exited.copy(end = PaneEnd.Lost("network lost")), true, null)
        assertEquals(PaneCover.Reconnecting, lost)
        assertFalse(lost.veils)
    }

    @Test
    fun onlyACoveredFollowerNeedsToTakeControl() {
        assertEquals(PaneCover.Follower, PaneCover.of(PaneLive(PaneRole.FOLLOWER, covered = true), true, null))
        assertEquals(PaneCover.Attaching, PaneCover.of(PaneLive(PaneRole.LEADER, covered = true), true, null))
        assertEquals(PaneCover.Attaching, PaneCover.of(PaneLive(covered = true), true, null))
        assertNull(PaneCover.of(PaneLive(PaneRole.FOLLOWER), true, null))
        assertNull(PaneCover.of(PaneLive(PaneRole.LEADER), true, null))
        assertNull(PaneCover.of(null, true, null))
        assertFalse((null as PaneCover?).veils)
        assertTrue(PaneCover.Follower.veils)
        assertTrue(PaneCover.Attaching.veils)
        assertTrue(PaneCover.Failed("failed").veils)
    }
}
