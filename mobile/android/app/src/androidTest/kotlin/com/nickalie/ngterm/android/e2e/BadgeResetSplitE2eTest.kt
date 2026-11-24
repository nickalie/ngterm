package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.ASK_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.SETTLE_MS
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A key typed on the phone into the session's split pane clears the session's unseen badge on the machine.
 *
 * The machine splits the session, so the phone shows a tab for each pane; the main pane's program raises the badge
 * and prints when it is gone, and the phone types into the split pane only.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class BadgeResetSplitE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aKeyTypedInTheSplitPaneClearsTheSessionsBadge() =
        with(driver) {
            openANewSession(session)
            run(
                "count() { ngtermctl tree --json | python3 -c 'import json,sys\n" +
                    "t=json.load(sys.stdin)[\"result\"][\"tree\"]\n" +
                    "print(sum(s.get(\"unseen\",0) for w in t[\"workspaces\"] for s in w[\"sessions\"]" +
                    " if s[\"name\"]==sys.argv[1]))' $session; }\n" +
                    "id=\$(ngtermctl tree --json | python3 -c 'import json,sys\n" +
                    "t=json.load(sys.stdin)[\"result\"][\"tree\"]\n" +
                    "print(next(s[\"id\"] for w in t[\"workspaces\"] for s in w[\"sessions\"]" +
                    " if s[\"name\"]==sys.argv[1]))' $session)\n" +
                    "ngtermctl session split on --target \"\$id\" >/dev/null\n" +
                    "sleep 2\n" +
                    "ngtermctl notify qa-split-badge --target \"\$id\" >/dev/null\n" +
                    "sleep 1\n" +
                    "echo \"badge-before=\$(count)\"\n" +
                    "until [ \"\$(count)\" = 0 ]; do sleep 1; done\n" +
                    "echo \"badge-after=\$(count)\"\n" +
                    "sleep 300\n",
            )
            awaitTerminal("badge-before=1")
            // a phone shows one pane of a split at a time, behind numbered tabs
            // a tab's label holds its pane's title too, once the pane's program sets one
            val mainTab = hasContentDescription(text(R.string.session_pane, 1, 2), substring = true)
            val splitTab = hasContentDescription(text(R.string.session_pane, 2, 2), substring = true)
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(splitTab) > 0 }
            screenshot("split-badge-raised")

            showTheSplitPane(splitTab)
            Thread.sleep(SETTLE_MS)
            compose.onNode(mainTab).performClick()
            Thread.sleep(SETTLE_MS)
            assertTrue(terminalText().contains("badge-before=1"))
            assertFalse("showing the split pane cleared the badge before a key", "badge-after=0" in terminalText())

            showTheSplitPane(splitTab)
            screenshot("split-focused")
            shell("input keyevent KEYCODE_ENTER")
            Thread.sleep(SETTLE_MS)
            compose.onNode(mainTab).performClick()
            awaitTerminal("badge-after=0")
            screenshot("split-badge-cleared")
        }

    // shows the split's second pane, takes control of it when the machine leads it, and gives it the input focus
    private fun SessionLinkDriver.showTheSplitPane(splitTab: SemanticsMatcher) {
        compose.onNode(splitTab).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { "badge-before" !in terminalText() }
        val cover = hasText(text(R.string.session_take_control))
        val covered = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(cover) > 0 } }.isSuccess
        if (covered) {
            compose.onAllNodesWithText(text(R.string.session_take_control))[0].performClick()
            compose.waitUntil(CONNECT_WAIT_MS) { nodes(cover) == 0 }
        }
        compose.onNode(hasContentDescription(text(R.string.terminal_description))).performClick()
        compose.waitForIdle()
    }
}
