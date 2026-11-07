package com.nickalie.ngterm.android.ui.chat

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.view.MotionEvent
import android.view.inspector.WindowInspector
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.core.app.ActivityOptionsCompat
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** System picker results and every upload outcome as the composer presents them. */
@RunWith(AndroidJUnit4::class)
class ComposerAttachCoverageTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theDocumentPickerReturnsAFileOrCancelsWithoutAttaching() {
        picker("File", Intent.ACTION_OPEN_DOCUMENT)
    }

    @Test
    fun theMediaPickerReturnsAMediaUriOrCancelsWithoutAttaching() {
        picker("Photo or video", android.provider.MediaStore.ACTION_PICK_IMAGES)
    }

    private fun picker(
        menu: String,
        action: String,
    ) {
        val registry = PickerRegistry()
        val owner =
            object : ActivityResultRegistryOwner {
                override val activityResultRegistry = registry
            }
        val picked = mutableListOf<Uri>()
        val enabled = mutableStateOf(false)
        compose.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                NgtermTheme { AttachButton(enabled.value, { picked += it }) }
            }
        }
        compose.onNodeWithContentDescription("Attach a file").assertIsNotEnabled()
        enabled.value = true
        compose.onNodeWithContentDescription("Attach a file").performClick()
        compose.onNodeWithText(menu).assertIsDisplayed()
        // Robolectric does not route touches between windows: deliver Android's outside event to the popup.
        // This follows MachineEntryTest's system-boundary helper.
        compose.runOnIdle {
            val outside = MotionEvent.obtain(0L, 0L, MotionEvent.ACTION_OUTSIDE, 0f, 0f, 0)
            try {
                WindowInspector.getGlobalWindowViews().last().dispatchTouchEvent(outside)
            } finally {
                outside.recycle()
            }
        }
        compose.onNodeWithText(menu).assertDoesNotExist()
        assertEquals(0, registry.request)
        compose.onNodeWithContentDescription("Attach a file").performClick()
        compose.onNodeWithText(menu).performClick()
        compose.onNodeWithText(menu).assertDoesNotExist()
        assertEquals(action, registry.intent.action)
        if (menu == "File") {
            assertEquals("*/*", registry.intent.type)
            assertEquals(listOf("*/*"), registry.intent.getStringArrayExtra(Intent.EXTRA_MIME_TYPES)?.toList())
        } else {
            // Both kinds of visual media are accepted, rather than a filter for just images or videos.
            assertEquals(null, registry.intent.type)
        }
        compose.runOnIdle { registry.dispatchResult(registry.request, Activity.RESULT_CANCELED, null) }
        assertTrue(picked.isEmpty())
        compose.onNodeWithContentDescription("Attach a file").performClick()
        compose.onNodeWithText(menu).performClick()
        val uri = Uri.parse("content://picked/shot.png")
        compose.runOnIdle { registry.dispatchResult(registry.request, Activity.RESULT_OK, Intent().setData(uri)) }
        assertEquals(listOf(uri), picked)
    }

    @Test
    fun unknownSizeStartsAtZeroAndSavingReplacesCancelWithAFullBar() {
        val upload = mutableStateOf<ChatUpload>(ChatUpload.Sending("notes.txt", 0, 0))
        var cancelled = 0
        compose.setContent { NgtermTheme { UploadLine(upload.value, { cancelled++ }, {}) } }
        compose.onNodeWithText("Uploading notes.txt").assertIsDisplayed()
        compose.onNodeWithText("0%").assertIsDisplayed()
        progress(0f)
        compose.onNodeWithContentDescription("Cancel upload").performClick()
        assertEquals(1, cancelled)
        upload.value = ChatUpload.Sending("notes.txt", 1, 4)
        compose.onNodeWithText("25%").assertIsDisplayed()
        progress(0.25f)
        upload.value = ChatUpload.Sending("notes.txt", 4, 4, saving = true)
        compose.onNodeWithText("Saving notes.txt").assertIsDisplayed()
        compose.onNodeWithText("25%").assertDoesNotExist()
        compose.onNodeWithContentDescription("Cancel upload").assertDoesNotExist()
        progress(1f)
    }

    private fun progress(value: Float) {
        compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo)).assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ProgressBarRangeInfo, ProgressBarRangeInfo(value, 0f..1f)),
        )
    }

    @Test
    fun unreadableAndRefusedUploadsAnnounceWhyAndCanBeDismissed() {
        val upload = mutableStateOf<ChatUpload>(ChatUpload.Failed("gone.txt", UploadProblem.Unreadable))
        var dismissed = 0
        compose.setContent { NgtermTheme { UploadLine(upload.value, {}, { dismissed++ }) } }
        compose.onNodeWithText("Couldn't attach gone.txt: the phone couldn't read it").assertIsDisplayed().assert(
            SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite),
        )
        compose.onNode(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo)).assertDoesNotExist()
        compose.onNodeWithContentDescription("Dismiss").performClick()
        assertEquals(1, dismissed)
        upload.value = ChatUpload.Failed("gone.txt", UploadProblem.Refused("disk full"))
        compose.onNodeWithText("Couldn't attach gone.txt: disk full").assertIsDisplayed()
        compose.onNodeWithContentDescription("Dismiss").performClick()
        assertEquals(2, dismissed)
    }

    private class PickerRegistry : ActivityResultRegistry() {
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
}
