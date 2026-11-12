package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneClipboard
import com.nickalie.ngterm.core.ffi.PaneRef
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import kotlin.coroutines.EmptyCoroutineContext

/**
 * A program's clipboard reads: asked, previewed, answered with each requested type, remembered per machine or by a
 * grant, and dismissed by the core.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ClipboardReadsTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var core: FakeMobileCore
    private val clipboard = MemoryClipboard().apply { text = "copied text" }
    private val pane = PaneRef("m1", "s1", "P1")
    private val copied = part("text/plain", "copied text")

    private fun TestScope.reads(
        trust: ClipboardTrust = ClipboardTrust(testStore(dir.root, "clipboard")),
        device: DeviceClipboard = clipboard,
    ): ClipboardReads {
        val connections = Connections(this, { 0L }) { sink -> FakeMobileCore(sink).also { core = it } }
        return ClipboardReads(backgroundScope, connections, trust, device, EmptyCoroutineContext)
    }

    private fun request(
        read: ClipboardRead,
        granted: Boolean = false,
    ) = core.emit(
        MobileEvent.ClipboardReadRequest(
            read.pane,
            read.readId,
            PaneClipboard.STANDARD,
            read.listOnly,
            read.mimes,
            read.list,
            granted,
            read.canRemember,
        ),
    )

    private fun request(
        id: ULong,
        pane: PaneRef = this.pane,
        granted: Boolean = false,
        canRemember: Boolean = false,
    ) = request(ClipboardRead(pane, id, listOnly = false, canRemember = canRemember), granted)

    /** The answers given, as (read id, reply) pairs that compare by their bytes. */
    private fun answers() =
        core.calls
            .filter { it.method == "answerClipboardRead" }
            .map { (it.args[3] as ULong) to (it.args[4] as ClipboardReply).key() }

    private fun answeredPanes() =
        core.calls
            .filter { it.method == "answerClipboardRead" }
            .map { PaneRef(it.args[0] as String, it.args[1] as String, it.args[2] as String) }

    // the store writes on its own threads: run the test's coroutines until what it wrote reaches them
    private suspend fun TestScope.settle(done: () -> Boolean) {
        repeat(SETTLE_ROUNDS) {
            advanceUntilIdle()
            if (done()) return
            withContext(Dispatchers.Default) { delay(SETTLE_MS) }
        }
        error("not settled")
    }

    // the preview of a read whose clipboard reads
    private suspend fun ClipboardReads.shown(read: ClipboardRead) = checkNotNull(preview(read))

    @Test
    fun aReadWaitsForTheUserAndAllowHandsOverWhatThePreviewShowed() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(7u)
            assertEquals(listOf(ClipboardRead(pane, 7u, listOnly = false)), reads.pending.value)
            assertEquals(emptyList<Any>(), answers())

            val read = reads.pending.value.single()
            assertEquals(listOf(copied.key()), reads.shown(read).contents.map { it.key() })
            clipboard.text = "copied later"
            reads.allow(read, reads.shown(read))
            assertEquals("the Allow hands over what was shown", listOf(7uL to allowed(copied)), answers())
            assertEquals(emptyList<ClipboardRead>(), reads.pending.value)
        }

    @Test
    fun denyRefusesAndAnEmptyClipboardIsAllowedAsNothing() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(1u)
            request(2u)
            reads.deny(reads.pending.value.first())
            clipboard.text = null
            reads.pending.value
                .single()
                .let { reads.allow(it, reads.shown(it)) }
            assertEquals(listOf(1uL to ClipboardReply.Deny.key(), 2uL to allowed()), answers())
        }

    @Test
    fun eachRequestedTypeTheClipboardHoldsIsHandedOverInOrderWithTheListing() =
        runTest {
            val png = ClipboardPart("image/png", byteArrayOf(-119, 80, 78, 71, 0))
            val html = part("text/html", "<b>hi</b>")
            clipboard.contents = listOf(html, copied, png)
            val reads = reads()
            advanceUntilIdle()
            request(
                ClipboardRead(pane, 1u, false, listOf("image/png", "application/x-missing", "text/plain"), list = true),
            )
            reads.pending.value
                .single()
                .let { reads.allow(it, reads.shown(it)) }
            assertEquals(
                listOf(1uL to allowed(png, copied, available = listOf("text/html", "text/plain", "image/png"))),
                answers(),
            )
        }

    @Test
    fun aListingOrAReadOfNoTypeIsAnsweredUnaskedWithNoData() =
        runTest {
            clipboard.contents = (1..20).map { part("application/x-$it", "$it") }
            val reads = reads()
            advanceUntilIdle()
            request(ClipboardRead(pane, 1u, listOnly = true, mimes = emptyList(), list = true))
            request(ClipboardRead(pane, 2u, listOnly = false, mimes = emptyList(), list = true))
            assertEquals(emptyList<ClipboardRead>(), reads.pending.value)
            runCurrent()
            val listed = (1..16).map { "application/x-$it" }
            assertEquals(
                "a listing names sixteen types at most",
                listOf(1uL to allowed(available = listed), 2uL to allowed(available = listed)),
                answers(),
            )
        }

    @Test
    fun aGrantedReadIsAnsweredUnaskedAndRememberIsKeptOnlyWhereItCanBe() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(1u, granted = true)
            assertEquals(emptyList<ClipboardRead>(), reads.pending.value)
            runCurrent()

            request(2u, canRemember = true)
            request(3u)
            val (rememberable, other) = reads.pending.value
            reads.allow(rememberable, reads.shown(rememberable), remember = true)
            reads.allow(other, reads.shown(other), remember = true)
            assertEquals(
                listOf(
                    1uL to allowed(copied),
                    2uL to allowed(copied, remember = true),
                    3uL to allowed(copied, remember = false),
                ),
                answers(),
            )
        }

    @Test
    fun alwaysAllowAnswersTheMachinesLaterReadsUnasked() =
        runTest {
            val trust = ClipboardTrust(testStore(dir.root, "clipboard"))
            val reads = reads(trust)
            advanceUntilIdle()
            request(1u)
            reads.pending.value
                .single()
                .let { reads.allowAlways(it, reads.shown(it)) }
            assertEquals(setOf("m1"), trust.machines.first { it.isNotEmpty() })

            request(2u)
            assertEquals("answered at once", emptyList<ClipboardRead>(), reads.pending.value)
            runCurrent()
            val other = PaneRef("m2", "s9", "P9")
            request(3u, other)
            assertEquals("another machine still asks", listOf(ClipboardRead(other, 3u, false)), reads.pending.value)
            assertEquals(listOf(1uL to allowed(copied), 2uL to allowed(copied)), answers())

            trust.revoke("m1")
            settle { reads.trusted.value.isEmpty() }
            request(4u)
            assertEquals(2, reads.pending.value.size)
        }

    @Test
    fun alwaysAllowAnswersTheMachinesReadsAlreadyWaiting() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            val sibling = PaneRef("m1", "s2", "P2")
            val other = PaneRef("m2", "s9", "P9")
            request(1u, canRemember = true)
            request(2u, sibling, canRemember = true)
            request(3u, other)
            reads.pending.value
                .first()
                .let { reads.allowAlways(it, reads.shown(it), remember = true) }
            assertEquals("another machine still asks", listOf(ClipboardRead(other, 3u, false)), reads.pending.value)
            runCurrent()
            assertEquals(
                "only the read answered keeps a grant",
                listOf(1uL to allowed(copied, remember = true), 2uL to allowed(copied)),
                answers(),
            )
            assertEquals(listOf(pane, sibling), answeredPanes())
        }

    @Test
    fun aReadTheCoreDismissesLeavesUnanswered() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(1u)
            request(2u)
            core.emit(MobileEvent.ClipboardReadDismiss(pane, 1u))
            assertEquals(listOf(ClipboardRead(pane, 2u, false)), reads.pending.value)
            core.emit(MobileEvent.ClipboardReadDismiss(PaneRef("m1", "s1", "other"), 2u))
            assertEquals("only its own pane's read goes", 1, reads.pending.value.size)
            assertEquals(emptyList<Any>(), answers())
        }

    @Test
    fun anUnaskedReadTakesItsSnapshotOffTheCoresThread() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(1u, granted = true)
            assertEquals("the core's thread never reads the clipboard", 0, clipboard.reads)
            assertEquals(emptyList<Any>(), answers())
            assertEquals(emptyList<ClipboardRead>(), reads.pending.value)
            runCurrent()
            assertEquals(1, clipboard.reads)
            assertEquals(listOf(1uL to allowed(copied)), answers())
        }

    @Test
    fun anUnaskedReadDismissedBeforeItsSnapshotGoesUnanswered() =
        runTest {
            reads()
            advanceUntilIdle()
            request(1u, granted = true)
            request(2u, granted = true)
            core.emit(MobileEvent.ClipboardReadDismiss(pane, 1u))
            runCurrent()
            assertEquals("only the dismissed read goes unread", 1, clipboard.reads)
            assertEquals(listOf(2uL to allowed(copied)), answers())
        }

    @Test
    fun anUnaskedReadDismissedWhileItsSnapshotIsTakenGoesUnanswered() =
        runTest {
            val dismissing =
                object : DeviceClipboard by clipboard {
                    override fun read(mimes: List<String>): List<ClipboardPart> {
                        core.emit(MobileEvent.ClipboardReadDismiss(pane, 1u))
                        return clipboard.read(mimes)
                    }
                }
            reads(device = dismissing)
            advanceUntilIdle()
            request(1u, granted = true)
            request(2u, granted = true)
            runCurrent()
            assertEquals("read 2 is not dismissed", listOf(2uL to allowed(copied)), answers())
        }

    @Test
    fun anUnaskedReadTheClipboardFailsIsRefusedAndLaterReadsGoOn() =
        runTest {
            var failing = true
            val flaky =
                object : DeviceClipboard by clipboard {
                    override fun read(mimes: List<String>): List<ClipboardPart> {
                        if (failing) {
                            failing = false
                            error("the provider failed")
                        }
                        return clipboard.read(mimes)
                    }
                }
            reads(device = flaky)
            advanceUntilIdle()
            request(1u, granted = true)
            request(2u, granted = true)
            runCurrent()
            assertEquals(listOf(1uL to ClipboardReply.Deny.key(), 2uL to allowed(copied)), answers())
        }

    @Test
    fun anAskedReadWhoseClipboardFailsToPreviewIsRefused() =
        runTest {
            val failing =
                object : DeviceClipboard by clipboard {
                    override fun read(mimes: List<String>): List<ClipboardPart> = error("the provider failed")
                }
            val reads = reads(device = failing)
            advanceUntilIdle()
            request(1u)
            request(2u)
            val read = reads.pending.value.first()
            assertNull("no snapshot to Allow", reads.preview(read))
            assertEquals(listOf(1uL to ClipboardReply.Deny.key()), answers())
            assertEquals("the next read still waits", listOf(2uL), reads.pending.value.map { it.readId })
        }

    private companion object {
        const val SETTLE_ROUNDS = 200
        const val SETTLE_MS = 10L
    }
}
