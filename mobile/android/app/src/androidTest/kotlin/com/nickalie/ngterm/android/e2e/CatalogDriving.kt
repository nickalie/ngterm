package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextReplacement
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.MACHINE

/** The steps the catalog e2e tests share, on top of [SessionLinkDriver]. */
internal object CatalogDriving {
    /**
     * Shell text that defines `shows TEXT`: it waits until the machine's tree shows TEXT. Five minutes outlast the
     * phone's waits in each window, and a failed run leaves no poller.
     */
    const val SHOWS =
        "shows() { n=0; until ngtermctl tree | grep -qF -- \"\$1\"; do " +
            "n=\$((n+1)); [ \$n -gt 300 ] && return 1; sleep 1; done; }\n"

    /** From the open session view to the machine's sessions list: the session view hides the destinations. */
    fun SessionLinkDriver.openTheMachine() {
        compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
        awaitText(text(R.string.nav_machines))
        compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
    }

    /**
     * Whether the sessions list holds a row that [matcher] matches, scrolled into view: the list composes only the
     * rows on the screen, and the machine keeps the sessions of the classes that ran before.
     */
    fun SessionLinkDriver.showsRow(matcher: SemanticsMatcher): Boolean =
        runCatching {
            compose.onAllNodes(hasScrollToNodeAction()).onFirst().performScrollToNode(matcher)
        }.isSuccess &&
            nodes(matcher) > 0

    fun SessionLinkDriver.awaitRow(name: String) = compose.waitUntil(CONNECT_WAIT_MS) { showsRow(hasText(name)) }

    fun SessionLinkDriver.awaitNoRow(name: String) = compose.waitUntil(CONNECT_WAIT_MS) { !showsRow(hasText(name)) }

    /** Scrolls the sessions list to its top, where its banners are. */
    fun SessionLinkDriver.toTheTop() {
        runCatching { compose.onAllNodes(hasScrollToNodeAction()).onFirst().performScrollToIndex(0) }
    }

    /** Opens Rename session on the row [from] and types [to] over its name, the dialog left open. */
    fun SessionLinkDriver.typeANewName(
        from: String,
        to: String,
    ) {
        awaitRow(from)
        compose.onNode(more(from)).performClick()
        compose.onNodeWithText(text(R.string.machine_sessions_rename)).performClick()
        compose.onNode(hasText(from) and hasAnyAncestor(isDialog())).performTextReplacement(to)
    }

    /** Taps Rename in the open Rename session dialog. */
    fun SessionLinkDriver.confirmTheRename() {
        compose.onNode(hasText(text(R.string.rename_session_save)) and hasAnyAncestor(isDialog())).performClick()
    }

    /** Renames the row [from] to [to] with Rename session, and waits until the list shows the new name alone. */
    fun SessionLinkDriver.renameOnThePhone(
        from: String,
        to: String,
    ) {
        typeANewName(from, to)
        confirmTheRename()
        awaitRow(to)
        awaitNoRow(from)
    }
}
