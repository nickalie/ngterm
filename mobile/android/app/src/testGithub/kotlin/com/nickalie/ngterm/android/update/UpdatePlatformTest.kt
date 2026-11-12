package com.nickalie.ngterm.android.update

import android.app.Application
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.Signature
import android.content.pm.SigningInfo
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File

/** The update's Android parts: reading APKs' signers, the installer session and the installer's answers. */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class UpdatePlatformTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val app = ApplicationProvider.getApplicationContext<Application>()
    private val certificate = byteArrayOf(1, 2, 3)

    @Suppress("DEPRECATION") // Android 8's signers
    private fun signed(sdk28: Boolean) {
        val info = shadowOf(app.packageManager).getInternalMutablePackageInfo(app.packageName)
        info.versionName = "0.2.3"
        if (sdk28) {
            info.signingInfo = SigningInfo().also { shadowOf(it).setSignatures(arrayOf(Signature(certificate))) }
        } else {
            info.signatures = arrayOf(Signature(certificate))
        }
    }

    @Test
    fun theInstalledAppsSignersAreItsCertificatesSha256() {
        signed(sdk28 = true)
        val (version, facts) = PackageApks(app).installed()
        assertEquals("0.2.3", version)
        assertEquals(app.packageName, facts.packageName)
        assertEquals(setOf(sha256Hex(certificate)), facts.signers)
        assertNull(PackageApks(app).archive(File(dir.root, "not.apk").apply { writeText("no") }))
    }

    @Test
    @Config(sdk = [27])
    fun android8ReadsTheSignersTheOldWay() {
        signed(sdk28 = false)
        assertEquals(setOf(sha256Hex(certificate)), PackageApks(app).installed().second.signers)
    }

    @Test
    @Suppress("DEPRECATION") // Robolectric's way to have the installer answer a committed session
    fun theApkGoesIntoAnInstallerSessionThatReportsBack() {
        val apk = File(dir.root, "ngterm-0.3.0-android.apk").apply { writeBytes(apkBytes("0.3.0")) }
        SessionInstaller(app).install(apk)
        val installer = app.packageManager.packageInstaller
        val session = installer.allSessions.single()
        assertEquals(app.packageName, session.appPackageName)
        // Android's installer answers through the session's status intent, which names the app's receiver (Robolectric
        // fills in no status: what the status means is theInstallersAnswersReadAsResults')
        shadowOf(installer).setSessionSucceeds(session.sessionId)
        val sent = shadowOf(app).broadcastIntents.last { it.action == ACTION_INSTALL_STATUS }
        assertEquals(InstallResultReceiver::class.java.name, sent.component?.className)
    }

    @Test
    fun theInstallersAnswersReadAsResults() {
        val status = { code: Int -> Intent(ACTION_INSTALL_STATUS).putExtra(PackageInstaller.EXTRA_STATUS, code) }
        val confirm = Intent("confirm")
        assertEquals(
            InstallResult.NeedsYou(confirm).confirm.action,
            (
                installResult(
                    status(PackageInstaller.STATUS_PENDING_USER_ACTION).putExtra(Intent.EXTRA_INTENT, confirm),
                ) as InstallResult.NeedsYou
            ).confirm.action,
        )
        assertEquals(InstallResult.Failed(null), installResult(status(PackageInstaller.STATUS_PENDING_USER_ACTION)))
        assertEquals(InstallResult.Done, installResult(status(PackageInstaller.STATUS_SUCCESS)))
        assertEquals(InstallResult.Aborted, installResult(status(PackageInstaller.STATUS_FAILURE_ABORTED)))
        assertEquals(
            InstallResult.Failed("INSTALL_FAILED_UPDATE_INCOMPATIBLE"),
            installResult(
                status(PackageInstaller.STATUS_FAILURE_INCOMPATIBLE)
                    .putExtra(PackageInstaller.EXTRA_STATUS_MESSAGE, "INSTALL_FAILED_UPDATE_INCOMPATIBLE"),
            ),
        )
        assertNull(installResult(Intent("other")))
        assertNull(installResult(null))
    }

    @Test
    fun theReceiversReachTheAppsUpdates() {
        val rig = UpdateRig(dir.root)
        val updates = rig.updates()
        (app as TestApplication).updates = updates
        InstallResultReceiver().onReceive(app, Intent("other"))
        InstallResultReceiver().onReceive(
            app,
            Intent(ACTION_INSTALL_STATUS).putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_SUCCESS),
        )
        assertEquals(UpdateState.Current, updates.state.value)

        SkipUpdateReceiver().onReceive(app, Intent(ACTION_SKIP_UPDATE))
        SkipUpdateReceiver().onReceive(app, Intent("other").putExtra("version", "0.3.0"))
        assertEquals(0, rig.cleared)
        SkipUpdateReceiver().onReceive(app, Intent(ACTION_SKIP_UPDATE).putExtra("version", "0.3.0"))
        assertEquals(1, rig.cleared)

        // an application without updates (the receivers' process holds none) is left alone
        app.updates = null
        SkipUpdateReceiver().onReceive(app, Intent(ACTION_SKIP_UPDATE).putExtra("version", "0.3.0"))
        InstallResultReceiver().onReceive(
            app,
            Intent(ACTION_INSTALL_STATUS).putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_SUCCESS),
        )
        assertEquals(1, rig.cleared)
    }

    @Test
    fun theGitHubBuildMakesItsUpdatesForTheInstalledVersion() {
        signed(sdk28 = true)
        val updates = appUpdates(app) as SelfUpdates
        assertEquals("0.2.3", updates.installed)
        assertEquals(UpdateState.Idle, updates.state.value)
        // Android's confirmation comes up as a task of its own; with no installer to show it, nothing breaks
        updates.installResult(InstallResult.NeedsYou(Intent("confirm")))
        val started = shadowOf(app).nextStartedActivity
        assertEquals("confirm", started.action)
        assertTrue(started.flags and Intent.FLAG_ACTIVITY_NEW_TASK != 0)
        shadowOf(app).checkActivities(true)
        updates.installResult(InstallResult.NeedsYou(Intent("confirm")))
        assertNull(shadowOf(app).nextStartedActivity)
    }
}
