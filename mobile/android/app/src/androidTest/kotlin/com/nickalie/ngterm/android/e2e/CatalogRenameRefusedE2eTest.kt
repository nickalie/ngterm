package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.e2e.CatalogDriving.SHOWS
import com.nickalie.ngterm.android.e2e.CatalogDriving.awaitRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.confirmTheRename
import com.nickalie.ngterm.android.e2e.CatalogDriving.openTheMachine
import com.nickalie.ngterm.android.e2e.CatalogDriving.renameOnThePhone
import com.nickalie.ngterm.android.e2e.CatalogDriving.showsRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.typeANewName
import com.nickalie.ngterm.android.e2e.SessionLinkDriver.Companion.CONNECT_WAIT_MS
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * Rename session on a session that the machine closed while its dialog was open: the dialog stays, keeps the typed
 * name, and shows under the field why the machine did not rename it.
 *
 * The machine closes the session [OPEN_DELAY_S] seconds after its tree shows a rename that the phone made from the
 * open list; the phone opens the dialog right after that rename and waits until the closed row leaves the list.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class CatalogRenameRefusedE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aRenameTheMachineRefusesShowsWhyInTheDialog() =
        with(driver) {
            val signal = "$session-signal"
            val go = "$session-go"
            val doomed = "$session-doomed"
            val typed = "$session-typed"
            openANewSession(session)
            run(
                "ngtermctl session new --no-select --name $signal >/dev/null\n" +
                    "id=\$(ngtermctl session new --no-select --name $doomed)\n" +
                    SHOWS +
                    "(shows $go && sleep $OPEN_DELAY_S && ngtermctl session close --target \"\$id\") " +
                    ">/dev/null 2>&1 &\n" +
                    "echo catalog-armed\n",
            )
            awaitTerminal("catalog-armed")
            openTheMachine()
            awaitRow(doomed)

            renameOnThePhone(signal, go)
            typeANewName(doomed, typed)
            val gone = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { !showsRow(hasText(doomed)) } }.isSuccess
            assertTrue("the machine never closed $doomed", gone)
            confirmTheRename()

            val inDialog = hasAnyAncestor(isDialog())
            val failed =
                runCatching {
                    compose.waitUntil(CONNECT_WAIT_MS) {
                        nodes(hasText(text(R.string.rename_session_save)) and inDialog) > 0 &&
                            nodes(hasText(NO_SUCH_SESSION, substring = true) and inDialog) > 0
                    }
                }.isSuccess
            assertTrue("the dialog never said why the rename failed", failed)
            assertTrue("the dialog dropped the typed name", nodes(hasText(typed) and inDialog) > 0)
            screenshot("catalog-rename-failed")
        }

    private companion object {
        /** Long enough for the phone to open the dialog and type the new name before the machine closes the row. */
        const val OPEN_DELAY_S = 8

        /** The machine's words for a session it does not hold. */
        const val NO_SUCH_SESSION = "no such session"
    }
}
