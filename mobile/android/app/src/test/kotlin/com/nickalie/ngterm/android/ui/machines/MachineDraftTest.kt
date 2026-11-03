package com.nickalie.ngterm.android.ui.machines

import com.nickalie.ngterm.android.data.StoredMachine
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MachineDraftTest {
    private val good = MachineDraft(label = "", host = "build-01.lan", port = "22", user = "nick", keyId = "k1")

    @Test
    fun aCompleteDraftHasNoProblemsAndBecomesItsMachine() {
        assertTrue(good.problems().isEmpty())
        assertEquals(
            StoredMachine("m1", "", "build-01.lan", 22, "nick", "k1"),
            good.copy(host = "  build-01.lan ", user = " nick").toMachine("m1"),
        )
    }

    @Test
    fun eachFieldSaysWhatIsWrongWithIt() {
        val cases =
            mapOf(
                good.copy(host = "") to FieldProblem.HostMissing,
                good.copy(host = "build 01") to FieldProblem.HostHasSpace,
                good.copy(host = "nick@build-01") to FieldProblem.HostHasUser,
                good.copy(host = "build-01:2222") to FieldProblem.HostHasPort,
                good.copy(port = "") to FieldProblem.PortInvalid,
                good.copy(port = "0") to FieldProblem.PortInvalid,
                good.copy(port = "65536") to FieldProblem.PortInvalid,
                good.copy(user = " ") to FieldProblem.UserMissing,
                good.copy(user = "ni ck") to FieldProblem.UserHasSpace,
                good.copy(keyId = "") to FieldProblem.KeyMissing,
            )
        for ((draft, problem) in cases) {
            assertEquals("$draft", mapOf(problem.field to problem), draft.problems())
        }
    }

    @Test
    fun anIpv6AddressIsAHostNotAHostWithAPort() {
        assertTrue(good.copy(host = "fe80::1").problems().isEmpty())
        assertTrue(good.copy(host = "2001:db8::42").problems().isEmpty())
    }

    @Test
    fun aMachineEditsAsTheDraftItWasMadeFrom() {
        val machine = StoredMachine("m1", "Build box", "build-01.lan", 2222, "nick", "k1")
        assertEquals(machine, MachineDraft.of(machine).toMachine("m1"))
    }
}
