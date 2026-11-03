package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.FakeMobileCore
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
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** A program's clipboard reads: asked, answered, remembered per machine, and dismissed by the core. */
@OptIn(ExperimentalCoroutinesApi::class)
class ClipboardReadsTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var core: FakeMobileCore
    private var clipboard: String? = "copied text"
    private val pane = PaneRef("m1", "s1", "P1")

    private fun TestScope.reads(
        trust: ClipboardTrust = ClipboardTrust(testStore(dir.root, "clipboard")),
    ): ClipboardReads {
        val connections = Connections(this, { 0L }) { sink -> FakeMobileCore(sink).also { core = it } }
        return ClipboardReads(backgroundScope, connections, trust) { clipboard }
    }

    private fun request(
        id: ULong,
        pane: PaneRef = this.pane,
        listOnly: Boolean = false,
    ) = core.emit(MobileEvent.ClipboardReadRequest(pane, id, PaneClipboard.STANDARD, listOnly))

    private fun answers() = core.calls.filter { it.method == "answerClipboardRead" }

    private fun answer(
        id: ULong,
        reply: ClipboardReply,
        pane: PaneRef = this.pane,
    ) = FakeCall("answerClipboardRead", listOf(pane.machineId, pane.sessionId, pane.paneId, id, reply))

    // the store writes on its own threads: run the test's coroutines until what it wrote reaches them
    private suspend fun TestScope.settle(done: () -> Boolean) {
        repeat(SETTLE_ROUNDS) {
            advanceUntilIdle()
            if (done()) return
            withContext(Dispatchers.Default) { delay(SETTLE_MS) }
        }
        error("not settled")
    }

    @Test
    fun aReadWaitsForTheUserAndAllowHandsOverTheClipboard() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(7u, listOnly = true)
            assertEquals(listOf(ClipboardRead(pane, 7u, listOnly = true)), reads.pending.value)
            assertEquals(emptyList<FakeCall>(), answers())

            reads.allow(reads.pending.value.single())
            assertEquals(listOf(answer(7u, ClipboardReply.Allow("copied text"))), answers())
            assertEquals(emptyList<ClipboardRead>(), reads.pending.value)
        }

    @Test
    fun denyGivesAnEmptyClipboardAndAnEmptyClipboardIsAllowedAsNone() =
        runTest {
            val reads = reads()
            advanceUntilIdle()
            request(1u)
            request(2u)
            reads.deny(reads.pending.value.first())
            clipboard = null
            reads.allow(reads.pending.value.single())
            assertEquals(
                listOf(answer(1u, ClipboardReply.Deny), answer(2u, ClipboardReply.Allow(null))),
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
            reads.allowAlways(reads.pending.value.single())
            assertEquals(setOf("m1"), trust.machines.first { it.isNotEmpty() })

            request(2u)
            assertEquals("answered at once", emptyList<ClipboardRead>(), reads.pending.value)
            val other = PaneRef("m2", "s9", "P9")
            request(3u, other)
            assertEquals("another machine still asks", listOf(ClipboardRead(other, 3u, false)), reads.pending.value)
            assertEquals(
                listOf(
                    answer(1u, ClipboardReply.Allow("copied text")),
                    answer(2u, ClipboardReply.Allow("copied text")),
                ),
                answers(),
            )

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
            request(1u)
            request(2u, sibling)
            request(3u, other)
            reads.allowAlways(reads.pending.value.first())
            assertEquals("another machine still asks", listOf(ClipboardRead(other, 3u, false)), reads.pending.value)
            assertEquals(
                listOf(
                    answer(1u, ClipboardReply.Allow("copied text")),
                    answer(2u, ClipboardReply.Allow("copied text"), sibling),
                ),
                answers(),
            )
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
            assertEquals(emptyList<FakeCall>(), answers())
        }

    private companion object {
        const val SETTLE_ROUNDS = 200
        const val SETTLE_MS = 10L
    }
}
