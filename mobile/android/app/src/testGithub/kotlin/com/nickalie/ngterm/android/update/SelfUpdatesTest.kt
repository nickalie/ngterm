package com.nickalie.ngterm.android.update

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import java.util.concurrent.CountDownLatch

/** The GitHub build's updates over fake GitHub, APK checks, installer and notifications. */
@RunWith(AndroidJUnit4::class)
class SelfUpdatesTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val rig by lazy { UpdateRig(dir.root) }

    private val offer = UpdateOffer("0.3.0", apkBytes("0.3.0").size.toLong(), "${RELEASE_PAGES}tag/v0.3.0")

    private fun released(vararg versions: String) {
        rig.http.releases(*versions.map { releaseJson(it) }.toTypedArray())
        versions.forEach(rig.http::assets)
    }

    private fun available(updates: AppUpdates) =
        awaitState(updates) {
            it is UpdateState.Available
        } as UpdateState.Available

    private fun failed(updates: AppUpdates) = awaitState(updates) { it is UpdateState.Failed } as UpdateState.Failed

    private fun settle(block: () -> Boolean) =
        runBlocking {
            withTimeout(5_000) {
                while (!block()) delay(10)
            }
        }

    /** The APK the installer got, which it gets just after the state says Installing. */
    private fun handedOver(): File {
        settle { rig.installed.isNotEmpty() }
        return rig.installed.single()
    }

    @Test
    fun aStartCheckAnnouncesANewVersionOnceAndKeepsTheBackgroundChecks() {
        released("0.3.0", "0.2.3")
        val updates = rig.updates()
        assertEquals("0.2.3", updates.installed)
        assertEquals(UpdateState.Idle, updates.state.value)
        updates.appStarted()
        assertEquals(UpdateState.Available(offer), available(updates))
        settle { runBlocking { rig.prefs.announced() } == "0.3.0" }
        assertEquals(listOf("0.3.0"), rig.announced)
        assertEquals(listOf(true), rig.schedule)

        // the next start finds it again, but it was announced already
        val again = rig.updates()
        again.appStarted()
        available(again)
        settle { rig.schedule.size == 2 }
        assertEquals(listOf("0.3.0"), rig.announced)
        // a newer one is announced
        released("0.3.1")
        assertTrue(runBlocking { again.checkInBackground() })
        assertEquals("0.3.1", available(again).offer.version)
        assertEquals(listOf("0.3.0", "0.3.1"), rig.announced)
    }

    @Test
    fun aManualCheckFindsTheNewestOrNothingWithoutAnnouncing() {
        released("0.2.3")
        val updates = rig.updates()
        updates.check()
        awaitState(updates) { it == UpdateState.Current }
        settle { rig.cleared == 1 }
        released("0.3.0")
        updates.check()
        available(updates)
        assertEquals(emptyList<String>(), rig.announced)
    }

    @Test
    fun withAutomaticChecksOffAStartNeitherChecksNorSchedules() {
        released("0.3.0")
        runBlocking { rig.prefs.setAutomatic(false) }
        val updates = rig.updates()
        updates.appStarted()
        settle { rig.schedule == listOf(false) }
        assertEquals(UpdateState.Idle, updates.state.value)
        assertEquals(emptyList<String>(), rig.http.fetched)

        updates.setAutomatic(true)
        settle { rig.schedule == listOf(false, true) }
        assertTrue(runBlocking { updates.automatic.first() })
        updates.setAutomatic(false)
        settle { rig.schedule == listOf(false, true, false) }
    }

    @Test
    fun aCheckThatCantReachGitHubSaysWhyAndCanBeTriedAgain() {
        rig.http.failure = IOException("no route to host")
        val updates = rig.updates()
        assertFalse(runBlocking { updates.checkInBackground() })
        assertEquals(UpdateState.Failed(UpdateFailure.Check("no route to host")), updates.state.value)
        rig.http.failure = null
        rig.http.bodies[RELEASES_API] = "not json".toByteArray()
        updates.check()
        assertTrue(failed(updates).failure is UpdateFailure.Check)
        released("0.3.0")
        updates.check()
        available(updates)
    }

    @Test
    fun aSkippedVersionIsntAnnouncedAgainButANewerOneIs() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        updates.opened(Intent(ACTION_SHOW_UPDATE))
        assertTrue(updates.prompt.value)
        updates.skip()
        assertEquals(UpdateState.Available(offer, skipped = true), updates.state.value)
        assertFalse(updates.prompt.value)
        settle { runBlocking { rig.prefs.skipped() } == "0.3.0" }

        val later = rig.updates()
        later.appStarted()
        assertTrue(available(later).skipped)
        assertEquals(emptyList<String>(), rig.announced)
        // the notification's Not now names its version, whatever the state
        later.skip("0.3.0")
        released("0.3.1")
        later.appStarted()
        settle { rig.announced == listOf("0.3.1") }
    }

    @Test
    fun installDownloadsVerifiesAndHandsTheApkToTheInstaller() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        updates.install()
        awaitState(updates) { it == UpdateState.Installing(offer) }
        val apk = handedOver()
        assertEquals("ngterm-0.3.0-android.apk", apk.name)
        assertArrayEquals(apkBytes("0.3.0"), apk.readBytes())
        assertTrue(rig.http.fetched.any { it.endsWith("/SHA256SUMS.minisig") })

        // the installer asks the user: the confirmation comes up, and Install brings it back
        val confirm = Intent("confirm")
        updates.installResult(InstallResult.NeedsYou(confirm))
        assertEquals(UpdateState.Installing(offer, waitingForYou = true), updates.state.value)
        updates.install()
        assertEquals(listOf(confirm, confirm), rig.launched)
        updates.installResult(InstallResult.Aborted)
        assertEquals(UpdateState.Available(offer), updates.state.value)
        updates.installResult(InstallResult.Failed("INSTALL_FAILED_UPDATE_INCOMPATIBLE"))
        assertEquals(
            UpdateState.Failed(UpdateFailure.Install("INSTALL_FAILED_UPDATE_INCOMPATIBLE"), offer),
            updates.state.value,
        )
        updates.installResult(InstallResult.Done)
        assertEquals(UpdateState.Current, updates.state.value)
    }

    @Test
    fun theNotificationsInstallOpensTheSheetChecksAndInstalls() {
        released("0.3.0")
        val updates = rig.updates()
        updates.opened(Intent("something else"))
        updates.opened(null)
        assertFalse(updates.prompt.value)
        updates.opened(Intent(ACTION_INSTALL_UPDATE))
        assertTrue(updates.prompt.value)
        awaitState(updates) { it is UpdateState.Installing }
        handedOver()
        updates.promptClosed()
        assertFalse(updates.prompt.value)
    }

    @Test
    fun anUpdateThatDoesntVerifyIsRefusedAndItsDownloadRemoved() {
        val refused = { problem: VerifyProblem ->
            val updates = rig.updates()
            updates.check()
            available(updates)
            updates.install()
            assertEquals(UpdateState.Failed(UpdateFailure.Verify(problem), offer), failed(updates))
            settle { rig.downloads.listFiles().isNullOrEmpty() }
        }
        released("0.3.0")
        val base = "${RELEASE_DOWNLOADS}v0.3.0/"
        rig.http.bodies[base + "SHA256SUMS.minisig"] = "signed 0.2.0".toByteArray()
        refused(VerifyProblem.Signature)
        rig.http.assets("0.3.0")
        rig.http.bodies[base + "SHA256SUMS"] = "${"0".repeat(64)}  other.apk\n".toByteArray()
        refused(VerifyProblem.Checksum)
        rig.http.assets("0.3.0")
        rig.http.bodies[base + "ngterm-0.3.0-android.apk"] = apkBytes("0.3.1").copyOf(apkBytes("0.3.0").size)
        refused(VerifyProblem.Checksum)
        rig.http.assets("0.3.0")
        rig.apks.archive = ApkFacts(APP_PACKAGE, 2_003, setOf(APP_KEY))
        refused(VerifyProblem.Package)
        rig.apks.archive = ApkFacts("com.example.other", 3_000, setOf(APP_KEY))
        refused(VerifyProblem.Package)
        rig.apks.archive = null
        refused(VerifyProblem.Package)
        rig.apks.archive = ApkFacts(APP_PACKAGE, 3_000, setOf("bb22"))
        refused(VerifyProblem.Signer)
        rig.apks.archive = ApkFacts(APP_PACKAGE, 3_000, emptySet())
        refused(VerifyProblem.Signer)
        assertEquals(emptyList<File>(), rig.installed)
    }

    @Test
    fun aFailedDownloadOrInstallSaysWhereItStoppedAndTryAgainResumes() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        rig.http.bodies.remove("${RELEASE_DOWNLOADS}v0.3.0/ngterm-0.3.0-android.apk")
        updates.install()
        assertEquals(UpdateFailure.Download("GitHub answered 404"), failed(updates).failure)

        rig.http.assets("0.3.0")
        rig.installFailure = SecurityException("not allowed to install")
        updates.install()
        awaitState(updates) { (it as? UpdateState.Failed)?.failure == UpdateFailure.Install("not allowed to install") }
        rig.installFailure = IOException("session gone")
        updates.install()
        awaitState(updates) { (it as? UpdateState.Failed)?.failure == UpdateFailure.Install("session gone") }

        rig.installFailure = null
        updates.install()
        awaitState(updates) { it is UpdateState.Installing }
    }

    @Test
    fun cancelStopsADownloadAndKeepsTheOffer() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        rig.http.gate = CompletableDeferred()
        updates.install()
        awaitState(updates) { it is UpdateState.Downloading }
        // busy: another check or install waits for this one
        updates.check()
        updates.install()
        updates.cancel()
        assertEquals(UpdateState.Available(offer), updates.state.value)
        settle { rig.downloads.listFiles().isNullOrEmpty() }
        assertTrue(runBlocking { updates.checkInBackground() })
        // nothing to cancel or skip any more
        updates.cancel()
        assertEquals(emptyList<File>(), rig.installed)
    }

    @Test
    fun aDownloadCancelledInTheMiddleOfAReadNeitherComesBackNorInstalls() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        val read = CountDownLatch(1)
        rig.http.stall = read
        updates.install()
        awaitState(updates) { it == UpdateState.Downloading(offer, offer.size / 2) }
        updates.cancel()
        read.countDown()
        settle { rig.http.ended.get() == 1 }
        settle { rig.downloads.listFiles().isNullOrEmpty() }
        assertEquals(UpdateState.Available(offer), updates.state.value)
        assertEquals(emptyList<File>(), rig.installed)
        val fetched = rig.http.fetched.size
        assertTrue(runBlocking { updates.checkInBackground() })
        assertTrue(rig.http.fetched.size > fetched)
    }

    @Test
    fun installAfterCancelWaitsForTheCancelledDownloadAndRunsOnce() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        val read = CountDownLatch(1)
        rig.http.stall = read
        updates.install()
        awaitState(updates) { it == UpdateState.Downloading(offer, offer.size / 2) }
        updates.cancel()
        updates.install()
        updates.install()
        read.countDown()
        awaitState(updates) { it == UpdateState.Installing(offer) }
        settle { rig.http.ended.get() == 2 }
        assertEquals(UpdateState.Installing(offer), updates.state.value)
        val apk = handedOver()
        assertArrayEquals(apkBytes("0.3.0"), apk.readBytes())
        assertTrue(rig.uncaught.isEmpty())
    }

    @Test
    fun cancelAfterInstallsQueuedBehindACancelledDownloadStopsTheOneThatRuns() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        val first = CountDownLatch(1)
        rig.http.stall = first
        updates.install()
        awaitState(updates) { it == UpdateState.Downloading(offer, offer.size / 2) }
        settle { rig.http.stall == null }
        updates.cancel()
        val second = CountDownLatch(1)
        rig.http.stall = second
        updates.install()
        updates.install()
        first.countDown()
        awaitState(updates) { it == UpdateState.Downloading(offer, offer.size / 2) }
        updates.cancel()
        updates.install()
        second.countDown()
        awaitState(updates) { it == UpdateState.Installing(offer) }
        settle { rig.http.ended.get() == 3 }
        // a download the cancel missed would empty the directory only after its read ends
        runBlocking { delay(200) }
        val apk = handedOver()
        assertArrayEquals(apkBytes("0.3.0"), apk.readBytes())
        assertTrue(rig.uncaught.isEmpty())
    }

    @Test
    fun aBackgroundCheckStoppedByTheSystemLeavesTheUpdaterUsable() {
        released("0.3.0")
        val updates = rig.updates()
        rig.http.fetchGate = CompletableDeferred()
        val worker = CoroutineScope(Dispatchers.Default).launch { updates.checkInBackground() }
        awaitState(updates) { it == UpdateState.Checking }
        runBlocking { worker.cancelAndJoin() }
        assertEquals(UpdateState.Idle, updates.state.value)
        rig.http.fetchGate = null
        updates.check()
        available(updates)
    }

    @Test
    fun aStoreThatCantBeReadOrWrittenStillChecksAnnouncesAndSkips() {
        val broken = UpdateRig(dir.root, BrokenStore())
        broken.http.releases(releaseJson("0.3.0"))
        val updates = broken.updates()
        updates.appStarted()
        assertEquals(UpdateState.Available(offer), available(updates))
        settle { broken.announced == listOf("0.3.0") }
        assertEquals(listOf(true), broken.schedule)
        assertTrue(runBlocking { updates.automatic.first() })
        // the switch can't be stored, so the schedule stays as the switch still shows it
        updates.setAutomatic(false)
        updates.skip()
        assertEquals(UpdateState.Available(offer, skipped = true), updates.state.value)
        assertTrue(runBlocking { updates.checkInBackground() })
        runBlocking {
            broken.scope.coroutineContext[Job]!!
                .children
                .forEach { it.join() }
        }
        assertEquals(listOf(true), broken.schedule)
        assertEquals(UpdateState.Available(offer, skipped = true), updates.state.value)
        assertEquals(listOf("0.3.0"), broken.announced)
        assertEquals(emptyList<Throwable>(), broken.uncaught)
    }

    @Test
    fun aDownloadThatCantBeStoredOrFailsWithoutWordsStillSaysSo() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        // the app's cache can't take the directory
        dir.root.setWritable(false)
        try {
            updates.install()
            assertEquals(UpdateFailure.Download("can't make ${rig.downloads.path}"), failed(updates).failure)
        } finally {
            dir.root.setWritable(true)
        }
        rig.installFailure = IOException()
        updates.install()
        awaitState(updates) { (it as? UpdateState.Failed)?.failure == UpdateFailure.Install("IOException") }
    }

    @Test
    fun whileItWorksTheBackgroundCheckAndInstallWait() {
        released("0.3.0")
        val updates = rig.updates()
        updates.check()
        available(updates)
        rig.http.gate = CompletableDeferred()
        updates.install()
        settle { rig.http.fetched.any { it.endsWith("/SHA256SUMS.minisig") } }
        val fetched = rig.http.fetched.size
        assertTrue(runBlocking { updates.checkInBackground() })
        assertEquals(fetched, rig.http.fetched.size)
        rig.http.gate!!.complete(Unit)
        awaitState(updates) { it is UpdateState.Installing }
        // the installer hasn't asked the user yet: Install has nothing to bring back
        updates.install()
        assertEquals(emptyList<Intent>(), rig.launched)
    }

    @Test
    fun anInstallResultWithNoUpdateUnderWayStillShowsTheConfirmation() {
        val updates = rig.updates()
        val confirm = Intent("confirm")
        updates.installResult(InstallResult.NeedsYou(confirm))
        updates.installResult(InstallResult.Aborted)
        updates.skip()
        assertEquals(listOf(confirm), rig.launched)
        assertEquals(UpdateState.Idle, updates.state.value)
        updates.install()
        awaitState(updates) { it is UpdateState.Failed }
    }

    @Test
    fun anInstalledBuildWithNoReleaseVersionTakesAnyRelease() {
        rig.apks.version = "dev"
        released("0.0.1")
        val updates = rig.updates()
        updates.check()
        assertEquals("0.0.1", available(updates).offer.version)
    }
}
