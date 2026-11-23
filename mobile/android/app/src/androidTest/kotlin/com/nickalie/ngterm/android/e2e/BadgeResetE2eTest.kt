package com.nickalie.ngterm.android.e2e

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith

/**
 * A key typed on the phone into the session's pane clears the session's unseen badge on the machine.
 *
 * Against a real ngterm machine, as [SessionLinkDriver] says; a class of its own, so it runs in an app process of
 * its own.
 */
@RunWith(AndroidJUnit4::class)
class BadgeResetE2eTest {
    private val driver = SessionLinkDriver()

    @get:Rule
    val rules: RuleChain = driver.rules

    @Test
    fun aKeyTypedOnThePhoneClearsTheSessionsBadge() =
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
                    "ngtermctl notify qa-badge --target \"\$id\" >/dev/null\n" +
                    "sleep 1\n" +
                    "echo \"badge-before=\$(count)\"\n" +
                    "read -r _\n" +
                    "sleep 2\n" +
                    "echo \"badge-after=\$(count)\"\n",
            )
            awaitTerminal("badge-before=1")
            screenshot("badge-raised")
            shell("input text x")
            shell("input keyevent KEYCODE_ENTER")
            awaitTerminal("badge-after=0")
            screenshot("badge-cleared")
        }
}
