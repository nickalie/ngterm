package com.nickalie.ngterm.android.update

import android.content.Intent
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONException
import java.io.File
import java.io.IOException

/** Where the GitHub build's updates come from and go: the network, the APK checks, the installer and the user. */
class UpdateParts(
    val prefs: UpdatePrefs,
    val http: UpdateHttp,
    val apks: ApkInspector,
    val verifier: UpdateVerifier,
    val installer: ApkInstaller,
    val notices: UpdateNotices,
    val schedule: UpdateSchedule,
    /** The directory the APK downloads into, emptied before each download and after an install. */
    val downloads: File,
    /** Shows Android's installer confirmation. */
    val launch: (Intent) -> Unit,
    val releasesUrl: String = RELEASES_API,
    val downloadBase: String = RELEASE_DOWNLOADS,
)

/**
 * [AppUpdates] from GitHub's releases: a check reads the release list for the newest complete Android release above
 * the installed version ([newestRelease]); the automatic one (at start, and [UpdateCheckWorker]'s) announces each
 * new version once, unless skipped. Install downloads the release's signed checksums and its APK, verifies them
 * ([UpdateVerifier]) and hands the APK to Android's installer, whose answers come back through [installResult].
 * State changes on [scope]; the network and the files are worked on [io].
 */
class SelfUpdates(
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher,
    private val parts: UpdateParts,
) : AppUpdates {
    override val installed: String = parts.apks.installed().first

    private val installedVersion = ReleaseVersion.parse(installed) ?: ReleaseVersion(0, 0, 0)
    private val current = MutableStateFlow<UpdateState>(UpdateState.Idle)
    private val showing = MutableStateFlow(false)

    @Volatile private var release: ReleaseDownload? = null

    @Volatile private var skipped: String? = null

    private var confirm: Intent? = null
    private var job: Job? = null

    override val state: StateFlow<UpdateState> = current.asStateFlow()
    override val automatic: Flow<Boolean> = parts.prefs.automatic
    override val prompt: StateFlow<Boolean> = showing.asStateFlow()

    private val busy: Boolean
        get() =
            when (current.value) {
                UpdateState.Idle, UpdateState.Current, is UpdateState.Available, is UpdateState.Failed -> false
                else -> true
            }

    override fun appStarted() {
        scope.launch {
            if (!busy) withContext(io) { parts.downloads.deleteRecursively() }
            if (parts.prefs.automatic.first()) {
                parts.schedule.start()
                if (!busy) runCheck(announce = true)
            } else {
                parts.schedule.stop()
            }
        }
    }

    override fun setAutomatic(on: Boolean) {
        scope.launch {
            parts.prefs.setAutomatic(on)
            if (on) parts.schedule.start() else parts.schedule.stop()
        }
    }

    override fun check() {
        if (busy) return
        job = scope.launch { runCheck(announce = false) }
    }

    /** The background check; false when GitHub couldn't be read, for the work to be retried. */
    suspend fun checkInBackground(): Boolean = busy || runCheck(announce = true)

    override fun install() {
        val now = current.value
        if (now is UpdateState.Installing) {
            confirm?.let(parts.launch)
            return
        }
        if (busy) return
        val target = release.takeIf { now is UpdateState.Available || (now is UpdateState.Failed && now.offer != null) }
        job =
            scope.launch {
                if (target != null) {
                    download(target)
                } else if (runCheck(announce = false)) {
                    release?.let { download(it) }
                }
            }
    }

    override fun cancel() {
        val now = current.value as? UpdateState.Downloading ?: return
        job?.cancel()
        current.value = UpdateState.Available(now.offer, now.offer.version == skipped)
    }

    override fun skip() {
        (current.value as? UpdateState.Available)?.let { skip(it.offer.version) }
    }

    /** Not now to [version], as the notification's action names it. */
    fun skip(version: String) {
        skipped = version
        parts.notices.clear()
        showing.value = false
        current.value.let { now ->
            if (now is UpdateState.Available && now.offer.version == version) current.value = now.copy(skipped = true)
        }
        scope.launch { parts.prefs.skip(version) }
    }

    override fun opened(intent: Intent?) {
        val action = intent?.action
        if (action != ACTION_SHOW_UPDATE && action != ACTION_INSTALL_UPDATE) return
        parts.notices.clear()
        showing.value = true
        if (action == ACTION_INSTALL_UPDATE) install()
    }

    override fun promptClosed() {
        showing.value = false
    }

    /** What Android's installer said of the install under way. */
    fun installResult(result: InstallResult) {
        val offer = release?.offer
        when (result) {
            is InstallResult.NeedsYou -> {
                confirm = result.confirm
                offer?.let { current.value = UpdateState.Installing(it, waitingForYou = true) }
                parts.launch(result.confirm)
            }

            InstallResult.Aborted -> {
                confirm = null
                offer?.let { current.value = UpdateState.Available(it, it.version == skipped) }
            }

            is InstallResult.Failed -> {
                confirm = null
                current.value = UpdateState.Failed(UpdateFailure.Install(result.message), offer)
            }

            InstallResult.Done -> {
                confirm = null
                current.value = UpdateState.Current
            }
        }
    }

    private suspend fun runCheck(announce: Boolean): Boolean {
        current.value = UpdateState.Checking
        val found =
            try {
                withContext(io) {
                    val json = parts.http.fetch(parts.releasesUrl, METADATA_LIMIT).decodeToString()
                    newestRelease(json, installedVersion, parts.downloadBase)
                }
            } catch (error: IOException) {
                current.value = UpdateState.Failed(UpdateFailure.Check(error.reason()))
                return false
            } catch (error: JSONException) {
                current.value = UpdateState.Failed(UpdateFailure.Check(error.reason()))
                return false
            }
        release = found
        if (found == null) {
            current.value = UpdateState.Current
            parts.notices.clear()
            return true
        }
        val offer = found.offer
        skipped = parts.prefs.skipped()
        current.value = UpdateState.Available(offer, offer.version == skipped)
        if (announce && offer.version != skipped && parts.prefs.announced() != offer.version) {
            parts.notices.available(offer)
            parts.prefs.announce(offer.version)
        }
        return true
    }

    private suspend fun download(target: ReleaseDownload) {
        val offer = target.offer
        current.value = UpdateState.Downloading(offer, 0)
        parts.notices.clear()
        try {
            withContext(io) { fetchVerifyInstall(target) }
        } catch (error: CancellationException) {
            withContext(io + NonCancellable) { parts.downloads.deleteRecursively() }
            throw error
        } catch (error: VerifyException) {
            fail(UpdateFailure.Verify(error.problem), offer)
        } catch (error: IOException) {
            val stage = current.value
            val failure =
                if (stage is UpdateState.Installing) {
                    UpdateFailure.Install(
                        error.reason(),
                    )
                } else {
                    UpdateFailure.Download(error.reason())
                }
            fail(failure, offer)
        } catch (error: SecurityException) {
            fail(UpdateFailure.Install(error.reason()), offer)
        }
    }

    private suspend fun fetchVerifyInstall(target: ReleaseDownload) {
        val offer = target.offer
        val dir = parts.downloads
        dir.deleteRecursively()
        if (!dir.mkdirs()) throw IOException("can't make ${dir.path}")
        val manifest = parts.http.fetch(target.sumsUrl, METADATA_LIMIT)
        val signature = parts.http.fetch(target.signatureUrl, SIGNATURE_LIMIT)
        val apk = File(dir, target.apk)
        var shown = -1L
        parts.http.download(target.apkUrl, apk, offer.size) { received ->
            val percent = received * PERCENT / offer.size
            if (percent != shown) {
                shown = percent
                current.value = UpdateState.Downloading(offer, received)
            }
        }
        current.value = UpdateState.Verifying(offer)
        parts.verifier.verify(target, apk, manifest, signature)
        current.value = UpdateState.Installing(offer)
        parts.installer.install(apk)
    }

    private suspend fun fail(
        failure: UpdateFailure,
        offer: UpdateOffer,
    ) {
        current.value = UpdateState.Failed(failure, offer)
        withContext(io) { parts.downloads.deleteRecursively() }
    }

    private companion object {
        const val PERCENT = 100
    }
}

private fun Exception.reason(): String = message ?: javaClass.simpleName
