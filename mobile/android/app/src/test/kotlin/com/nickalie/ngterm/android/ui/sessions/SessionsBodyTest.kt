package com.nickalie.ngterm.android.ui.sessions

import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.RemoteSession
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionsBodyTest {
    private fun sessions(
        link: Link = Link.Connected,
        listed: List<RemoteSession>? = null,
        failure: String? = null,
        attached: Set<String> = emptySet(),
        ending: Set<String> = emptySet(),
    ) = MachineSessions(testMachine(), link, listed, failure, attached, offline = false, ending = ending)

    @Test
    fun theLinkDecidesUntilTheMachineIsConnected() {
        assertEquals(SessionsBody.NotConnected(false), sessions(Link.Offline).body())
        assertEquals(SessionsBody.NotConnected(true), sessions(Link.Offline).copy(offline = true).body())
        assertEquals(SessionsBody.Connecting, sessions(Link.Connecting).body())
        assertEquals(SessionsBody.Connecting, sessions(Link.HostKeyUnknown("SHA256:x")).body())
        assertEquals(SessionsBody.Reconnecting(42), sessions(Link.Reconnecting(42)).body())
        assertEquals(SessionsBody.Unreachable("refused"), sessions(Link.Unreachable("refused")).body())
        assertEquals(SessionsBody.HostKeyChanged, sessions(Link.HostKeyChanged("a", "b")).body())
        // a listing kept from before doesn't hide that the machine is down
        assertEquals(
            SessionsBody.Unreachable("refused"),
            sessions(Link.Unreachable("refused"), listed = listOf(fakeSession("s1"))).body(),
        )
    }

    @Test
    fun aConnectedMachineShowsItsListingOrWhyThereIsNone() {
        assertEquals(SessionsBody.Reading, sessions().body())
        assertEquals(SessionsBody.ListFailed("no answer"), sessions(failure = "no answer").body())
        val missing = "/bin/sh: 1: ngtermctl: not found"
        assertEquals(SessionsBody.MissingCli(missing), sessions(failure = missing).body())
        assertEquals(SessionsBody.Empty(null), sessions(listed = emptyList()).body())
        // a failed refresh keeps the listing before it, and says so
        assertEquals(SessionsBody.Empty("no answer"), sessions(listed = emptyList(), failure = "no answer").body())
        val body = sessions(listed = listOf(fakeSession("s1")), failure = "no answer").body()
        assertEquals("no answer", (body as SessionsBody.Sessions).failure)
    }

    @Test
    fun attachedSessionsComeFirstAndCompanionsNameTheirParent() {
        val listed =
            listOf(
                fakeSession("a", name = "api"),
                fakeSession("a2", name = "api review", parent = "a"),
                fakeSession("b", name = "build"),
                fakeSession("c", name = "orphan", parent = "gone"),
            )
        val body = sessions(listed = listed, attached = setOf("b"), ending = setOf("a")).body() as SessionsBody.Sessions
        assertEquals(listOf("b"), body.attached.map { it.session.id })
        assertTrue(body.attached.single().attached)
        assertEquals(listOf("a", "a2", "c"), body.others.map { it.session.id })
        assertEquals(listOf(null, "api", "gone"), body.others.map { it.companionOf })
        assertEquals(listOf(true, false, false), body.others.map { it.ending })
    }

    @Test
    fun companionsSitUnderTheirParentInTheCoresOrderAndGoWithTheirFamily() {
        // the core lists every companion after all the parents
        val listed =
            listOf(
                fakeSession("a", name = "api"),
                fakeSession("b", name = "build"),
                fakeSession("a2", name = "api review", parent = "a"),
            )

        fun body(attached: Set<String>) = sessions(listed = listed, attached = attached).body() as SessionsBody.Sessions
        val none = body(emptySet())
        assertEquals(listOf("a", "a2", "b"), none.others.map { it.session.id })
        assertEquals(listOf(null, "api", null), none.others.map { it.companionOf })
        // attaching the parent, or only its companion, takes the whole family to the Attached group
        for (attached in listOf(setOf("a"), setOf("a2"))) {
            val body = body(attached)
            assertEquals(listOf("a", "a2"), body.attached.map { it.session.id })
            assertEquals(listOf("a", "a2").map { it in attached }, body.attached.map { it.attached })
            assertEquals(listOf("b"), body.others.map { it.session.id })
        }
    }

    @Test
    fun aMissingNgtermctlIsTheShellsWordsForIt() {
        assertTrue(isMissingCli("/bin/sh: 1: ngtermctl: not found"))
        assertTrue(isMissingCli("bash: line 1: ngtermctl: command not found"))
        assertTrue(isMissingCli("env: 'ngtermctl': No such file or directory"))
        assertFalse(isMissingCli("build-01 gave no answer; nothing was created"))
        assertFalse(isMissingCli("session not found"))
    }
}
