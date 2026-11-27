package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.e2e.CatalogDriving.SHOWS
import com.nickalie.ngterm.android.e2e.CatalogDriving.awaitNoRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.awaitRow
import com.nickalie.ngterm.android.e2e.CatalogDriving.openTheMachine
import com.nickalie.ngterm.android.e2e.CatalogDriving.renameOnThePhone
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
                    SHOWS +
                    "(shows $opened && id=\$(ngtermctl session new --no-select --name $made) && " +
                    "shows $seen && ngtermctl session rename --target \"\$id\" $renamed) >/dev/null 2>&1 &\n" +
                    "echo catalog-armed\n",
            )
            awaitTerminal("catalog-armed")
            openTheMachine()
            awaitRow(signal)

            renameOnThePhone(signal, opened)
            awaitRow(made)
            screenshot("catalog-made")
            renameOnThePhone(opened, seen)
            awaitRow(renamed)
            awaitNoRow(made)
            screenshot("catalog-renamed")
        }
}
