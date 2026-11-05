package com.nickalie.ngterm.android.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import java.util.Locale

/** When the account line says a weekly limit resets: a weekday and a time within six days, else a date. */
class AccountLineTest {
    private val zone = ZoneId.of("Europe/Vilnius")

    // Thursday 2026-11-05 12:00 UTC
    private val now = Instant.parse("2026-11-05T12:00:00Z")

    @Test
    fun aResetWithinSixDaysIsItsWeekdayAndTimeInTheZone() {
        // 14:00 UTC is 16:00 in Vilnius
        assertEquals("Mon 16:00", resetsAt(Instant.parse("2026-11-09T14:00:00Z"), now, zone, Locale.UK))
        assertEquals("Thu 16:00", resetsAt(Instant.parse("2026-11-05T14:00:00Z"), now, zone, Locale.UK))
    }

    @Test
    fun aLaterResetIsItsDate() {
        assertEquals("14 Nov 2026", resetsAt(Instant.parse("2026-11-14T14:00:00Z"), now, zone, Locale.UK))
    }
}
