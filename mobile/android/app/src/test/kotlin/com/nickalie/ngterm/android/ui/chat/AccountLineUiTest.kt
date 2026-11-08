package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatUsage
import org.junit.Assert.assertNotEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.temporal.ChronoUnit
import java.util.TimeZone

/**
 * Accounts can arrive without an email or reset time, and low usage is still announced as one sentence; a reset reads
 * in the device's zone as of its clock.
 */
@RunWith(AndroidJUnit4::class)
class AccountLineUiTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun usageAloneAndLowUsageWithAnEmailReadAsOneSentence() {
        val account = mutableStateOf(ChatBody.Account(null, ChatUsage(12u, null)))
        compose.setContent {
            NgtermTheme {
                AccountLine(account.value, now = Instant.parse("2026-11-05T12:00:00Z"), zone = ZoneId.of("UTC"))
            }
        }
        compose.onNodeWithContentDescription("12% of the weekly limit used.").assertIsDisplayed()
        account.value = ChatBody.Account("dev@example.com", ChatUsage(0u, null))
        compose
            .onNodeWithContentDescription("Signed in as dev@example.com. 0% of the weekly limit used.")
            .assertIsDisplayed()
        account.value = ChatBody.Account("dev@example.com", ChatUsage(100u, null))
        compose
            .onNodeWithContentDescription("Signed in as dev@example.com. 100% of the weekly limit used.")
            .assertIsDisplayed()
    }

    @Test
    fun theResetReadsInTheDevicesZoneAsOfItsClock() {
        val device = TimeZone.getDefault()
        // a zone far from UTC, so a reset read in UTC (or as of another clock) reads differently
        val zone = ZoneId.of("Pacific/Kiritimati")
        TimeZone.setDefault(TimeZone.getTimeZone(zone))
        try {
            val at = Instant.now().plus(Duration.ofDays(2)).truncatedTo(ChronoUnit.SECONDS)
            val resets = resetsAt(at, Instant.now(), zone)
            assertNotEquals(resetsAt(at, Instant.now(), ZoneOffset.UTC), resets)
            assertNotEquals(resetsAt(at, Instant.EPOCH, zone), resets)
            val account = ChatBody.Account("dev@example.com", ChatUsage(40u, at.epochSecond.toULong()))
            compose.setContent { NgtermTheme { AccountLine(account) } }
            val spoken = "Signed in as dev@example.com. 40% of the weekly limit used, resets $resets."
            compose.onNodeWithContentDescription(spoken).assertIsDisplayed()
        } finally {
            TimeZone.setDefault(device)
        }
    }
}
