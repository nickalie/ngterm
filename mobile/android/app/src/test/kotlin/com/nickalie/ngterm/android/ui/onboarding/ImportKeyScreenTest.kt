package com.nickalie.ngterm.android.ui.onboarding

import android.app.Activity
import android.content.ContentResolver
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.net.Uri
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextLayoutResult
import androidx.core.app.ActivityOptionsCompat
import androidx.test.core.app.ApplicationProvider
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.shadows.ShadowContentResolver
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream

/** Secret drafts, field errors and Android's document-picker outcomes, without replacing the screen. */
@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "w411dp-h891dp")
class ImportKeyScreenTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val registry = KeyPickerRegistry()
    private val owner =
        object : ActivityResultRegistryOwner {
            override val activityResultRegistry = registry
        }
    private val state = mutableStateOf(OnboardingStep.Import())
    private val imports = mutableListOf<Pair<ByteArray, String?>>()
    private var unreadable = 0
    private var back = 0

    private fun show() {
        compose.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                NgtermTheme {
                    ImportKeyScreen(
                        state.value.busy,
                        state.value.problem,
                        { bytes, phrase -> imports += bytes to phrase },
                        { unreadable++ },
                        { back++ },
                    )
                }
            }
        }
    }

    private fun field(label: String) = compose.onNode(hasText(label) and hasSetTextAction())

    private fun passphraseText(text: String) =
        SemanticsMatcher.expectValue(SemanticsProperties.EditableText, AnnotatedString(text))

    private fun pick(input: InputStream) {
        val uri = Uri.parse("content://keys/private-key")
        shadowOf(compose.activity.contentResolver).registerInputStream(uri, input)
        compose.onNodeWithText("Choose file").performScrollTo().performClick()
        compose.runOnIdle { registry.dispatchResult(registry.request, Activity.RESULT_OK, Intent().setData(uri)) }
    }

    @Test
    fun pasteAndPassphraseReachImportAndTheVisibilityToggleDoesNotChangeTheSecret() {
        show()
        field("Private key").performTextReplacement("private key text")
        field("Passphrase").performScrollTo().performTextReplacement("secret phrase")
        field("Passphrase").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Password))
        compose.onNodeWithText("Show passphrase").performScrollTo().performClick()
        val layouts = mutableListOf<TextLayoutResult>()
        field("Passphrase").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        assertEquals(
            "secret phrase",
            layouts
                .single()
                .layoutInput.text.text,
        )
        compose.onNodeWithText("Hide passphrase").performClick()
        layouts.clear()
        field("Passphrase").performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        assertEquals(
            "•".repeat("secret phrase".length),
            layouts
                .single()
                .layoutInput.text.text,
        )
        field("Passphrase").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Password))
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        assertArrayEquals("private key text".toByteArray(), imports.single().first)
        assertEquals("secret phrase", imports.single().second)
        compose.onNodeWithContentDescription("Back").performClick()
        assertEquals(1, back)
        compose.runOnIdle { state.value = OnboardingStep.Import(busy = true) }
        compose.onNodeWithText("Private key").assertIsNotEnabled()
        compose.onNodeWithText("Passphrase").assertIsNotEnabled()
        compose.onNodeWithText("Choose file").assertIsNotEnabled()
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        assertEquals(1, imports.size)
    }

    @Test
    fun everyImportProblemAppearsUnderItsOwnFieldAndPreservesTheDraft() {
        show()
        field("Private key").performTextReplacement("draft key")
        field("Passphrase").performScrollTo().performTextReplacement("draft phrase")
        val cases =
            listOf(
                ImportProblem.NeedsPassphrase to "This key is protected: enter its passphrase.",
                ImportProblem.WrongPassphrase to "That passphrase doesn't open this key.",
                ImportProblem.NotAKey("unsupported format") to
                    "This isn't a private key ngterm can use: unsupported format",
                ImportProblem.NotStored("disk full") to "The key couldn't be stored: disk full",
                ImportProblem.Empty to "Paste a private key or choose its file.",
                ImportProblem.Unreadable to "That file couldn't be read. Choose another, or paste the key.",
            )
        cases.forEach { (problem, words) ->
            compose.runOnIdle { state.value = OnboardingStep.Import(problem = problem) }
            compose.onNodeWithText(words).performScrollTo().assertIsDisplayed()
            val phraseProblem = problem == ImportProblem.NeedsPassphrase || problem == ImportProblem.WrongPassphrase
            val error = SemanticsMatcher.keyIsDefined(SemanticsProperties.Error)
            field(if (phraseProblem) "Passphrase" else "Private key").assert(error)
            field(if (phraseProblem) "Private key" else "Passphrase").assert(
                SemanticsMatcher.keyNotDefined(SemanticsProperties.Error),
            )
        }
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        assertArrayEquals("draft key".toByteArray(), imports.single().first)
        assertEquals("draft phrase", imports.single().second)
    }

    @Test
    fun cancellingThePickerDoesNothingAndChosenBytesSurviveImportWipingUntilReplacedByTyping() {
        show()
        compose.onNodeWithText("Choose file").performScrollTo().performClick()
        assertEquals(Intent.ACTION_OPEN_DOCUMENT, registry.intent.action)
        assertEquals("*/*", registry.intent.type)
        assertEquals(listOf("*/*"), registry.intent.getStringArrayExtra(Intent.EXTRA_MIME_TYPES)?.toList())
        compose.runOnIdle { registry.dispatchResult(registry.request, Activity.RESULT_CANCELED, null) }
        assertTrue(imports.isEmpty())
        assertEquals(0, unreadable)
        field("Passphrase").performScrollTo().performTextReplacement("file phrase")
        val file = TrackedKeyInput("chosen key".toByteArray())
        pick(file)
        assertTrue(file.closed)
        assertArrayEquals("chosen key".toByteArray(), imports.single().first)
        assertEquals("file phrase", imports.single().second)
        // AppKeys wipes the copy handed to it. Retrying must still read the chosen file's original bytes.
        imports.single().first.fill(0)
        compose
            .onNodeWithText("The key from the chosen file is ready. Typing here uses the text instead.")
            .performScrollTo()
            .assertIsDisplayed()
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        assertArrayEquals("chosen key".toByteArray(), imports.last().first)
        pick(TrackedKeyInput("replacement key".toByteArray()))
        assertArrayEquals("replacement key".toByteArray(), imports.last().first)
        field("Private key").performScrollTo().performTextReplacement("pasted instead")
        compose
            .onNodeWithText("The key from the chosen file is ready. Typing here uses the text instead.")
            .assertDoesNotExist()
        compose.onNodeWithText("Import key").performScrollTo().performClick()
        assertArrayEquals("pasted instead".toByteArray(), imports.last().first)
    }

    @Test
    fun unreadableRevokedAndOversizeFilesAreRefusedAndTheExactLimitIsAccepted() {
        show()
        listOf(IOException("provider offline"), SecurityException("permission revoked")).forEach { error ->
            val input = TrackedKeyInput(byteArrayOf(), error)
            pick(input)
            assertTrue(input.closed)
        }
        val oversized = TrackedKeyInput(ByteArray(KEY_LIMIT + 1))
        pick(oversized)
        assertTrue(oversized.closed)
        assertEquals(3, unreadable)
        assertTrue(imports.isEmpty())
        pick(TrackedKeyInput(ByteArray(KEY_LIMIT) { 1 }))
        assertEquals(KEY_LIMIT, imports.single().first.size)
        pick(TrackedKeyInput(byteArrayOf()))
        assertTrue(imports.last().first.isEmpty())
    }

    @Test
    fun privateDraftsAreNotSavedAndTheSecureWindowFlagIsScopedToTheScreen() {
        val restoration = StateRestorationTester(compose)
        val visible = mutableStateOf(true)
        restoration.setContent {
            NgtermTheme {
                if (visible.value) ImportKeyScreen(false, null, { _, _ -> }, {}, {})
            }
        }
        field("Private key").performTextReplacement("secret key")
        field("Passphrase").performScrollTo().performTextReplacement("secret phrase")
        field("Passphrase").assert(passphraseText("•".repeat("secret phrase".length)))
        compose.runOnIdle {
            assertTrue(compose.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        }
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("secret key").assertDoesNotExist()
        // The field is masked, so its semantic text is bullets: check the value it holds, not the secret's text.
        field("Passphrase").assert(passphraseText(""))
        compose.runOnIdle { visible.value = false }
        compose.runOnIdle {
            assertFalse(compose.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        }
    }

    @Test
    fun aWrappedApplicationContextCanShowAndDisposeTheScreenWithoutAWindow() {
        val visible = mutableStateOf(true)
        val context = ContextWrapper(ApplicationProvider.getApplicationContext<Context>())
        compose.setContent {
            CompositionLocalProvider(LocalContext provides context, LocalActivityResultRegistryOwner provides owner) {
                NgtermTheme {
                    if (visible.value) ImportKeyScreen(false, null, { _, _ -> }, {}, {})
                }
            }
        }
        compose.onNodeWithText("Private key").assertIsDisplayed()
        compose.runOnIdle { visible.value = false }
        compose.onNodeWithText("Private key").assertDoesNotExist()
    }

    @Test
    @Config(shadows = [CrashedProviderResolver::class])
    fun aProviderThatDiedBeforeOpeningTheStreamReportsUnreadable() {
        show()
        pick(TrackedKeyInput(byteArrayOf()))
        assertEquals(1, unreadable)
        assertTrue(imports.isEmpty())
    }

    /** Android returns a null stream when a document provider has just crashed. */
    @Implements(ContentResolver::class)
    class CrashedProviderResolver : ShadowContentResolver() {
        @Implementation
        override fun openInputStream(uri: Uri): InputStream? = null
    }

    private class KeyPickerRegistry : ActivityResultRegistry() {
        var request = 0
        lateinit var intent: Intent

        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            request = requestCode
            intent = contract.createIntent(ApplicationProvider.getApplicationContext(), input)
        }
    }

    private class TrackedKeyInput(
        bytes: ByteArray,
        private val failure: Exception? = null,
    ) : ByteArrayInputStream(bytes) {
        var closed = false

        override fun read(
            buffer: ByteArray,
            offset: Int,
            length: Int,
        ): Int {
            failure?.let { throw it }
            return super.read(buffer, offset, length)
        }

        override fun close() {
            closed = true
            super.close()
        }
    }

    private companion object {
        const val KEY_LIMIT = 64 * 1024
    }
}
