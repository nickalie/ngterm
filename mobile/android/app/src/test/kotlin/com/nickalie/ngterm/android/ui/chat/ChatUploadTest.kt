package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.IOException

/**
 * A file the composer attaches: on its way with its progress, its path into the prompt, cancelled (not once the machine
 * has it all), or refused.
 */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class ChatUploadTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var bed: ChatBed
    private lateinit var chat: ChatViewModel

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        bed = ChatBed(dir.root)
        bed.attach("p1")
        bed.show("p1")
        bed.agent("p1")
        eventually { bed.chat("p1") != null }
        chat = bed.chat("p1")!!
    }

    @After
    fun tearDown() {
        bed.close()
        Dispatchers.resetMain()
    }

    private fun upload() = chat.state.value.upload

    @Test
    fun anUploadShowsItsProgressThenPutsItsPathAtTheCursor() {
        chat.composer.edit(TextFieldValue("Look at this", TextRange(12)))
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["uploadFile"] = gate
        chat.attach("shot.png", 4) { byteArrayOf(1, 2, 3, 4) }
        eventually { (upload() as? ChatUpload.Sending)?.sent == 2L }
        assertEquals(ChatUpload.Sending("shot.png", 2, 4), upload())
        gate.complete(Unit)
        eventually { upload() == null }
        assertEquals("Look at this /state/uploads/1/shot.png ", chat.composer.draft.text)
        assertEquals(
            "shot.png",
            bed.test.core.uploads
                .single()
                .first,
        )
    }

    @Test
    fun aSecondAttachWaitsForTheFirst() {
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["uploadFile"] = gate
        chat.attach("a.txt", 1) { byteArrayOf(1) }
        chat.attach("b.txt", 1) { byteArrayOf(2) }
        gate.complete(Unit)
        eventually { upload() == null }
        assertEquals(
            listOf("a.txt"),
            bed.test.core.uploads
                .map { it.first },
        )
    }

    @Test
    fun cancellingStopsTheUploadAndLeavesThePromptAlone() {
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["uploadFile"] = gate
        chat.attach("big.bin", 8) { ByteArray(8) }
        eventually { upload() is ChatUpload.Sending }
        chat.cancelUpload()
        assertNull(upload())
        gate.complete(Unit)
        assertTrue(
            bed.test.core.uploads
                .isEmpty(),
        )
        assertEquals("", chat.composer.draft.text)
    }

    @Test
    fun onceEveryByteHasGoneTheUploadIsSavingAndCancelHoldsOff() {
        chat.composer.edit(TextFieldValue("See", TextRange(3)))
        val saving = CompletableDeferred<Unit>()
        bed.test.core.holds["uploadFile.saving"] = saving
        chat.attach("notes.txt", 4) { byteArrayOf(1, 2, 3, 4) }
        eventually { (upload() as? ChatUpload.Sending)?.saving == true }
        assertEquals(ChatUpload.Sending("notes.txt", 4, 4, saving = true), upload())
        // the machine has the whole file: a cancel couldn't take it back, so the upload runs to its path
        chat.cancelUpload()
        assertEquals(ChatUpload.Sending("notes.txt", 4, 4, saving = true), upload())
        saving.complete(Unit)
        eventually { upload() == null }
        assertEquals("See /state/uploads/1/notes.txt ", chat.composer.draft.text)
    }

    @Test
    fun aRefusalUnreadableOrOversizedFileSaysWhyUntilSeen() {
        bed.test.core.failNext("uploadFile", MobileException.Remote("disk full"))
        chat.attach("a.txt", 1) { byteArrayOf(1) }
        eventually { upload() is ChatUpload.Failed }
        assertEquals(ChatUpload.Failed("a.txt", UploadProblem.Refused("disk full")), upload())
        chat.uploadShown()
        assertNull(upload())

        chat.attach("gone.txt", null) { throw IOException("revoked") }
        eventually { upload() is ChatUpload.Failed }
        assertEquals(ChatUpload.Failed("gone.txt", UploadProblem.Unreadable), upload())

        chat.attach("huge.mov", UPLOAD_CAP + 1) { error("an oversized file is never read") }
        assertEquals(ChatUpload.Failed("huge.mov", UploadProblem.TooLarge), upload())
    }

    @Test
    fun aPathWithASpaceGoesInQuoted() {
        assertEquals("\"/state/uploads/1/my shot.png\"", promptPath("/state/uploads/1/my shot.png"))
        assertEquals("/state/uploads/1/shot.png", promptPath("/state/uploads/1/shot.png"))
    }
}
