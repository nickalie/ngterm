package com.nickalie.ngterm.android.e2e

import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A pick bound to the session the phone presents (`ngtermctl pick --session`) shows as the phone's sheet over it: the
 * prompt and the items, and a tap answers the origin's waiting `ngtermctl pick` with the item.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class SessionPickE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aSessionBoundPickShowsOnThePhoneAndATapAnswersIt() =
        with(driver) {
            openANewSession(session)
            run(pickScript(PROMPT))
            awaitText(PROMPT)
            screenshot("pick-shown")
            compose.onNodeWithText("beta").performClick()
            awaitTerminal(PICKED_BETA)
        }

    companion object {
        const val PROMPT = "QaPick"

        /** What [pickScript] prints once its pick answered with `beta`, which the script's own text does not hold. */
        const val PICKED_BETA = "answered-beta"

        /** Opens a pick of `alpha` and `beta` titled [prompt] on the script's session; prints how it was answered. */
        fun pickScript(
            prompt: String,
            before: String = "",
        ): String =
            before +
                "printf 'alpha\\nbeta\\n' | ngtermctl pick --prompt $prompt --session \"\$NGTERM_SESSION_ID\" " +
                "> qa-pick.json\n" +
                "grep -q picked qa-pick.json && grep -q beta qa-pick.json && printf '%s-%s\\n' answered beta\n" +
                "sleep 300\n"
    }
}
