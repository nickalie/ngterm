package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A headless machine's sessions list follows its catalog live: a session made and then renamed on the machine shows
 * in the open list with no refresh, and Rename session on the phone renames it there.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class CatalogListE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun theListFollowsTheMachineLiveAndRenameSessionRenamesThere() =
        with(driver) {
            val made = "$session-made"
            val renamed = "$session-renamed"
            val phone = "$session-phone"
            openANewSession(session)
            // the machine's changes wait until the list is open, so only the catalog can bring them
            run(
                "(sleep 15; id=\$(ngtermctl session new --no-select --name $made); sleep 5; " +
                    "ngtermctl session rename --target \"\$id\" $renamed) >/dev/null 2>&1 &\n" +
                    "echo catalog-armed\n",
            )
            awaitTerminal("catalog-armed")
            // the session view fills the screen: Back shows the destinations again
            compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
            awaitText(text(R.string.nav_machines))
            compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
            awaitText(MACHINE)
            compose.onNodeWithText(MACHINE).performClick()
            awaitText(session)
            awaitText(made)
            screenshot("catalog-made")
            awaitText(renamed)
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(made)) == 0 }

            compose.onNode(more(renamed)).performClick()
            compose.onNodeWithText(text(R.string.machine_sessions_rename)).performClick()
            compose.onNode(hasText(renamed) and hasAnyAncestor(isDialog())).performTextReplacement(phone)
            compose.onNode(hasText(text(R.string.rename_session_save)) and hasAnyAncestor(isDialog())).performClick()
            awaitText(phone)
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(renamed)) == 0 }
            screenshot("catalog-renamed")
        }
}
