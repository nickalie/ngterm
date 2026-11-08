package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatUsage
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.time.Instant
import java.time.ZoneId

/** Accounts can arrive without an email or reset time, and low usage is still announced as one sentence. */
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
    fun theLineCanUseTheDeviceClockAndZone() {
        compose.setContent { NgtermTheme { AccountLine(ChatBody.Account("dev@example.com", null)) } }
        compose.onNodeWithContentDescription("Signed in as dev@example.com.").assertIsDisplayed()
    }
}
