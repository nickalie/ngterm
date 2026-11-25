package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasImeAction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.input.ImeAction
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import org.junit.Assert.assertEquals

/** The pick and ask steps the pick e2e tests share, on top of [SessionLinkDriver]. */
internal object PickDriving {
    private const val PROMPT_SETTLE_MS = 2_000L

    /** A JSON item array with a subtitle and three statuses: a glyph for active and blocked, none for idle. */
    const val ITEMS_JSON =
        "[{\"id\":\"alpha\",\"label\":\"alpha\",\"subtitle\":\"qa-subtitle\",\"status\":\"active\"}," +
            "{\"id\":\"beta\",\"label\":\"beta\",\"status\":\"blocked\"}," +
            "{\"id\":\"gamma\",\"label\":\"gamma\",\"status\":\"idle\"}]"

    /** Shell text that writes [ITEMS_JSON] to `qa-items.json`. */
    const val WRITE_ITEMS = "cat > qa-items.json <<'QAEOF'\n$ITEMS_JSON\nQAEOF\n"

    /**
     * Writes [ITEMS_JSON] to `qa-items.json` and the scripts' shell helpers to `qa-lib.sh`: `p ARGS` picks from the
     * items on the script's session, `u` keeps the id a `--no-block` open prints (`{"id":…}`), `m A B` prints `A-B`
     * (a marker the script's own text does not hold) and `w FAMILY ID FILE NEEDLE` waits for a result holding NEEDLE
     * (a resolved result is read once: the read that sees it keeps it in FILE). A script typed whole must fit the
     * screen, so the scripts source this one.
     */
    const val PRELUDE =
        WRITE_ITEMS +
            "cat > qa-lib.sh <<'QAEOF'\n" +
            "p(){ ngtermctl pick --session \"\$NGTERM_SESSION_ID\" \"\$@\" <qa-items.json; }\n" +
            "u(){ grep -o '[0-9A-F-]\\{36\\}'; }\n" +
            "m(){ printf '%s-%s\\n' \"\$1\" \"\$2\"; }\n" +
            "w(){ until ngtermctl \$1 result \"\$2\" >\"\$3\" 2>/dev/null; grep -q \"\$4\" \"\$3\"; " +
            "do sleep 1; done; }\n" +
            "QAEOF\n" +
            "printf '%s-%s\\n' prelude ready\n"

    /** `printf` of `<a>-<b>`: a marker the script's own text does not hold. */
    fun mark(
        a: String,
        b: String,
    ): String = "printf '%s-%s\\n' $a $b\n"

    /** The pick's search field: the one text field whose keyboard action is Go. */
    val searchField: SemanticsMatcher = hasSetTextAction() and hasImeAction(ImeAction.Go)

    /** Runs [PRELUDE] in the session's shell, then [script], which sources its helpers. */
    fun SessionLinkDriver.runWithHelpers(script: String) {
        run(PRELUDE)
        awaitTerminal("prelude-ready")
        // the shell drops keys typed before its next prompt is up
        Thread.sleep(PROMPT_SETTLE_MS)
        run(". ./qa-lib.sh\n$script")
    }

    fun SessionLinkDriver.awaitGone(text: String) {
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) == 0 }
    }

    fun SessionLinkDriver.assertNotShown(text: String) {
        assertEquals("$text is on screen", 0, nodes(hasText(text)))
    }

    /** Back from a session's view, when it shows: the bar's destinations are under it. */
    fun SessionLinkDriver.leaveTheSessionView() {
        val back = compose.onAllNodesWithContentDescription(text(R.string.action_back))
        if (back.fetchSemanticsNodes().isNotEmpty()) back[0].performClick()
        compose.waitForIdle()
    }

    /** Opens the session [name] from the Sessions list. */
    fun SessionLinkDriver.openFromSessions(name: String) {
        compose.onAllNodesWithText(text(R.string.nav_sessions))[0].performClick()
        awaitText(name)
        compose.onAllNodesWithText(name)[0].performClick()
    }
}
