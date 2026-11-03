package com.nickalie.ngterm.android.ui.theme

import com.nickalie.ngterm.android.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The look an agent status gets when its origin sets no color or shape (the origin sends them only as overrides). */
class AgentStateTest {
    @Test
    fun theOriginsWireNamesAreKnown() {
        assertEquals(AgentState.Active, AgentState.fromWire("active"))
        assertEquals(AgentState.Blocked, AgentState.fromWire("blocked"))
        assertEquals(AgentState.Completed, AgentState.fromWire(" Completed "))
        assertEquals(AgentState.Idle, AgentState.fromWire("idle"))
        assertNull(AgentState.fromWire("Working"))
        assertNull(AgentState.fromWire(null))
    }

    @Test
    fun theStatesSayWhatTheyMeanAndLookApart() {
        assertEquals(R.string.agent_active, AgentState.Active.word)
        assertEquals(R.string.agent_blocked, AgentState.Blocked.word)
        assertEquals(R.string.agent_completed, AgentState.Completed.word)
        // waiting is the attention tone, glyph and color both
        assertEquals(StatusTone.Attention.shape, AgentState.Blocked.shape)
        assertEquals(DarkStatus.attention, AgentState.Blocked.color(DarkStatus))
        // no two states share a glyph, so none rests on its color
        assertEquals(
            AgentState.entries.size,
            AgentState.entries
                .map { it.shape }
                .toSet()
                .size,
        )
        assertNotEquals(AgentState.Active.color(LightStatus), AgentState.Blocked.color(LightStatus))
    }
}
