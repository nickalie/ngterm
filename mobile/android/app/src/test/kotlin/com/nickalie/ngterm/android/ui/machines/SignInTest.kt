package com.nickalie.ngterm.android.ui.machines

import android.content.ClipboardManager
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performFirstLinkClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitDescription
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.SIGN_IN_UNFINISHED
import com.nickalie.ngterm.core.fake.TailscaleSignIn
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** A machine that asks the user to sign in (Tailscale SSH's `check`), end to end on the fake core. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp")
class SignInTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    // the links the app handed to the browser
    private val opened = mutableListOf<String>()

    private val browser =
        object : UriHandler {
            override fun openUri(uri: String) {
                opened += uri
            }
        }

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        val key = test.seedKey()
        test.core.signIns["build-01.lan"] = TailscaleSignIn
        test.seedMachine(testMachine(keyId = key.id))
    }

    @After
    fun tearDown() = test.close()

    private fun app() {
        compose.setContent {
            NgtermTheme {
                CompositionLocalProvider(LocalUriHandler provides browser) {
                    NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = NavLayout.Bar)
                }
            }
        }
        compose.awaitText("build-01 asks you to sign in")
    }

    @Test
    fun theLinkOpensFromTheButtonOrTheMessageAndCopiesAndThePromptGoesOnceSignedIn() {
        app()
        compose.onNodeWithContentDescription("Waiting for sign-in").assertExists()
        compose.onNodeWithText("Sign in at its link to connect").assertExists()
        compose.onNodeWithText("Open link").performClick()
        compose.onNodeWithText(TailscaleSignIn.message).performFirstLinkClick()
        assertEquals(listOf(TailscaleSignIn.url, TailscaleSignIn.url), opened)
        compose.onNodeWithText("Copy link").performClick()
        compose.awaitText("Copied")
        val clipboard =
            ApplicationProvider.getApplicationContext<Context>().getSystemService(ClipboardManager::class.java)
        assertEquals(TailscaleSignIn.url, clipboard.primaryClip?.getItemAt(0)?.text)
        // the user signed in in the browser: the connect goes on and the prompt goes with it
        test.core.finishSignIn("m1")
        compose.awaitDescription("Connected")
        compose.waitForIdle()
        compose.onNodeWithText("build-01 asks you to sign in").assertDoesNotExist()
    }

    @Test
    fun dontConnectCancelsTheConnectAndLeavesTheMachineNotConnected() {
        app()
        compose.onNodeWithText("Don't connect").performClick()
        compose.awaitText("Connect")
        compose.onNodeWithContentDescription("Offline").assertExists()
        assertTrue(test.core.waitingForSignIn.isEmpty())
        assertTrue(opened.isEmpty())
    }

    @Test
    fun aSignInNotCompletedInTimeSaysWhy() {
        app()
        test.core.finishSignIn("m1", signedIn = false)
        compose.awaitText(SIGN_IN_UNFINISHED)
        compose.onNodeWithText("build-01 asks you to sign in").assertDoesNotExist()
    }
}
