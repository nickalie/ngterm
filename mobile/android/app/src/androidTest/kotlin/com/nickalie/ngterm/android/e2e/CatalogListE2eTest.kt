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
 * The machine makes each change only once its own tree shows a rename that the phone made from the open list, so
 * the open list can get the change only from the catalog, and the machine sees the phone's renames.
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
            val signal = "$session-signal"
            val opened = "$session-opened"
            val seen = "$session-seen"
            val made = "$session-made"
            val renamed = "$session-renamed"
            openANewSession(session)
            run(
                "ngtermctl session new --no-select --name $signal >/dev/null\n" +
                    "(until ngtermctl tree | grep -qF -- $opened; do sleep 1; done; " +
                    "id=\$(ngtermctl session new --no-select --name $made); " +
                    "until ngtermctl tree | grep -qF -- $seen; do sleep 1; done; " +
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
            awaitText(signal)

            renameOnThePhone(signal, opened)
            awaitText(made)
            screenshot("catalog-made")
            renameOnThePhone(opened, seen)
            awaitText(renamed)
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(made)) == 0 }
            screenshot("catalog-renamed")
        }

    private fun SessionLinkDriver.renameOnThePhone(
        from: String,
        to: String,
    ) {
        compose.onNode(more(from)).performClick()
        compose.onNodeWithText(text(R.string.machine_sessions_rename)).performClick()
        compose.onNode(hasText(from) and hasAnyAncestor(isDialog())).performTextReplacement(to)
        compose.onNode(hasText(text(R.string.rename_session_save)) and hasAnyAncestor(isDialog())).performClick()
        awaitText(to)
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(from)) == 0 }
    }
}
