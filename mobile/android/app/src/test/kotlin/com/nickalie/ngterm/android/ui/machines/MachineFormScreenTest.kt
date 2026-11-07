package com.nickalie.ngterm.android.ui.machines

import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/** Validation, key selection and saved fields as the user encounters them in the form. */
@RunWith(RobolectricTestRunner::class)
class MachineFormScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val created = AppKey("created", "ssh-ed25519 created", "SHA256:created", false)
    private val imported = AppKey("imported", "ssh-ed25519 imported", "SHA256:imported", true)
    private var saved: MachineDraft? = null

    private fun field(label: String) = compose.onNode(hasText(label) and hasSetTextAction())

    @Test
    fun errorsAppearAfterLeavingAFieldAndSaveRefusesUntilTheyAreCorrected() {
        compose.setContent {
            NgtermTheme {
                MachineFormScreen(MachineDraft(), false, listOf(created), null, { saved = it }, {}, {})
            }
        }
        compose.onNodeWithText("Enter the machine's hostname or IP address.").assertDoesNotExist()
        field("Host").performClick()
        field("Port").performClick()
        compose.onNodeWithText("Enter the machine's hostname or IP address.").assertExists()
        field("Host").performClick()
        field("Host").performTextReplacement("user@host")
        compose.onNodeWithText("Put the user in User, not in the host.").assertExists()
        field("Host").performTextReplacement("host:22")
        compose.onNodeWithText("Put the port in Port, not in the host.").assertExists()
        field("Host").performTextReplacement("two hosts")
        compose.onNodeWithText("A hostname has no spaces.").assertExists()
        field("Host").performTextReplacement("host.lan")
        field("Port").performTextReplacement("0")
        field("User").performScrollTo().performTextReplacement("two users")
        compose.onNodeWithText("Save").performClick()
        assertNull(saved)
        compose.onNodeWithText("Enter a port from 1 to 65535.").assertExists()
        compose.onNodeWithText("A user name has no spaces.").assertExists()
        field("Port").performScrollTo().performTextReplacement("2x2!")
        field("User").performScrollTo().performTextReplacement("nick")
        field("Name").performScrollTo().performTextReplacement("Build server")
        compose.onNodeWithText("Save").performClick()
        assertEquals(MachineDraft("Build server", "host.lan", "22", "nick", created.id), saved)
    }

    @Test
    fun missingKeysRefuseSaveAndDoNotOfferAPublicKey() {
        compose.setContent {
            NgtermTheme {
                MachineFormScreen(MachineDraft(host = "host", user = "nick"), false, emptyList(), null, {
                    saved = it
                }, {}, {})
            }
        }
        compose.onNodeWithText("Show public key").assertDoesNotExist()
        compose.onNodeWithText("Save").performClick()
        compose.onNodeWithText("Choose the key to sign in with.").performScrollTo().assertExists()
        assertNull(saved)
    }

    @Test
    fun selectedKeyAndAllEditedFieldsSurviveRestorationAndThePublicKeyUsesThatSelection() {
        val restoration = StateRestorationTester(compose)
        var shown: String? = null
        var closed = false
        restoration.setContent {
            NgtermTheme {
                MachineFormScreen(
                    MachineDraft("old", "old.lan", "22", "old", imported.id),
                    true,
                    listOf(created, imported),
                    null,
                    { saved = it },
                    { closed = true },
                    { shown = it },
                )
            }
        }
        compose.onNodeWithText("SHA256:imported").performScrollTo().assertIsSelected()
        compose.onNodeWithText("SHA256:created").performScrollTo().performClick()
        field("Name").performScrollTo().performTextReplacement("new")
        field("Host").performScrollTo().performTextReplacement("new.lan")
        field("Port").performScrollTo().performTextReplacement("2222")
        field("User").performScrollTo().performTextReplacement("newuser")
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("SHA256:created").performScrollTo().assertIsSelected()
        compose.onNodeWithText("Show public key").performScrollTo().performClick()
        assertEquals(created.id, shown)
        compose
            .onNodeWithText(
                "None yet: the first connection asks you to confirm it.",
            ).performScrollTo()
            .assertExists()
        compose.onNodeWithText("Save").performClick()
        assertEquals(MachineDraft("new", "new.lan", "2222", "newuser", created.id), saved)
        compose.onNodeWithContentDescription("Close").performClick()
        assertEquals(true, closed)
    }
}
