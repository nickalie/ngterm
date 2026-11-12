package com.nickalie.ngterm.android.ui.updates

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.update.UpdateFailure
import com.nickalie.ngterm.android.update.UpdateOffer
import com.nickalie.ngterm.android.update.UpdateState
import com.nickalie.ngterm.android.update.VerifyProblem
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The update panel says where updating stands in words and offers what can be done next. */
@RunWith(AndroidJUnit4::class)
class UpdatePanelTest {
    @get:Rule val compose = createComposeRule()

    private val offer = UpdateOffer("0.3.0", 38_000_000, "https://github.com/nickalie/ngterm/releases/tag/v0.3.0")
    private val shown = mutableStateOf<UpdateState>(UpdateState.Idle)
    private val calls = mutableListOf<String>()

    private val actions =
        UpdateActions(
            onCheck = { calls += "check" },
            onInstall = { calls += "install" },
            onCancel = { calls += "cancel" },
            onSkip = { calls += "skip" },
            onNotes = { calls += "notes:$it" },
        )

    private var started = false

    private fun show(state: UpdateState) {
        shown.value = state
        if (!started) compose.setContent { NgtermTheme { UpdatePanel("0.2.3", shown.value, actions) } }
        started = true
        compose.waitForIdle()
    }

    private fun click(text: String) = compose.onNodeWithText(text).performClick()

    private fun text(text: String) = compose.onNodeWithText(text, substring = true).assertIsDisplayed()

    @Test
    fun beforeAndAfterACheckItOffersToCheck() {
        show(UpdateState.Idle)
        text("Version 0.2.3")
        text("Not checked since ngterm started.")
        click("Check now")
        show(UpdateState.Checking)
        text("Checking for updates")
        show(UpdateState.Current)
        text("Up to date")
        text("0.2.3 is the newest release.")
        click("Check now")
        assertEquals(listOf("check", "check"), calls)
    }

    @Test
    fun anAvailableUpdateShowsTheStepItsSizeAndItsActions() {
        show(UpdateState.Available(offer))
        text("ngterm 0.3.0 is available")
        compose.onNodeWithContentDescription("From version 0.2.3 to 0.3.0").assertIsDisplayed()
        text("download from GitHub")
        click("Install")
        click("Release notes")
        click("Not now")
        assertEquals(listOf("install", "notes:${offer.notesUrl}", "skip"), calls)

        show(UpdateState.Available(offer, skipped = true))
        text("You skipped this version")
        compose.onNodeWithText("Not now").assertDoesNotExist()
    }

    @Test
    fun aDownloadShowsItsProgressAndCanBeCancelled() {
        show(UpdateState.Downloading(offer, 19_000_000))
        text("Downloading ngterm 0.3.0")
        text(" of ")
        click("Cancel download")
        show(UpdateState.Verifying(offer))
        text("Checking the download")
        show(UpdateState.Installing(offer))
        text("Android's installer takes it from here")
        compose.onNodeWithText("Open installer").assertDoesNotExist()
        show(UpdateState.Installing(offer, waitingForYou = true))
        text("Confirm the update in Android's installer")
        click("Open installer")
        assertEquals(listOf("cancel", "install"), calls)
    }

    @Test
    fun aFailureSaysWhatWentWrongAndTryAgainRepeatsThatStep() {
        show(UpdateState.Failed(UpdateFailure.Check("timeout")))
        text("Couldn't check for updates")
        text("GitHub couldn't be reached: timeout")
        click("Try again")
        val failures =
            mapOf(
                UpdateFailure.Download("reset") to "The download stopped: reset",
                UpdateFailure.Install("INSTALL_FAILED") to "Android's installer refused it: INSTALL_FAILED",
                UpdateFailure.Install(null) to "Android's installer refused it.",
                UpdateFailure.Verify(VerifyProblem.Signature) to "aren't signed with ngterm's release key",
                UpdateFailure.Verify(VerifyProblem.Checksum) to "doesn't match the checksum",
                UpdateFailure.Verify(VerifyProblem.Package) to "isn't a newer version of this app",
                UpdateFailure.Verify(VerifyProblem.Signer) to "signed with a different key",
            )
        for ((failure, words) in failures) {
            show(UpdateState.Failed(failure, offer))
            text("ngterm 0.3.0 wasn't installed")
            text(words)
        }
        click("Try again")
        click("Release notes")
        assertEquals(listOf("check", "install", "notes:${offer.notesUrl}"), calls)
    }
}
