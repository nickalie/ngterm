package com.nickalie.ngterm.android.update

import android.content.Intent
import androidx.datastore.core.CorruptionException
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.core.ffi.ReleaseException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger

const val APP_PACKAGE = "com.nickalie.ngterm.android"
const val APP_KEY = "aa11"

/** The APK bytes of release [version] in the fakes. */
fun apkBytes(version: String) = "the APK of $version".toByteArray()

fun sha256Hex(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") {
        "%02x".format(it)
    }

/** One release as GitHub's API lists it, with its assets at their own download URLs. */
fun releaseJson(
    version: String,
    prerelease: Boolean = false,
    draft: Boolean = false,
    assets: List<Pair<String, Long>> =
        listOf(
            "ngterm-$version-android.apk" to apkBytes(version).size.toLong(),
            "SHA256SUMS" to 100L,
            "SHA256SUMS.minisig" to 300L,
        ),
    base: String = RELEASE_DOWNLOADS,
): String {
    val listed =
        assets.joinToString(",") { (name, size) ->
            """{"name":"$name","size":$size,"browser_download_url":"${base}v$version/$name"}"""
        }
    return """{"tag_name":"v$version","draft":$draft,"prerelease":$prerelease,""" +
        """"html_url":"${RELEASE_PAGES}tag/v$version","assets":[$listed]}"""
}

/**
 * [UpdateHttp] over canned bodies by URL. [gate] holds a download until completed and [fetchGate] a fetch; [stall]
 * blocks the next download halfway, deaf to cancellation as a socket read is; [failure] fails every read.
 */
class FakeHttp : UpdateHttp {
    val bodies = ConcurrentHashMap<String, ByteArray>()
    val fetched = CopyOnWriteArrayList<String>()
    var failure: IOException? = null
    var gate: CompletableDeferred<Unit>? = null
    var fetchGate: CompletableDeferred<Unit>? = null
    var stall: CountDownLatch? = null
    val ended = AtomicInteger()

    fun releases(vararg json: String) {
        bodies[RELEASES_API] = "[${json.joinToString(",")}]".toByteArray()
    }

    /** Serves release [version]'s assets: its APK, and a checksum list and signature the fake checksum reads. */
    fun assets(version: String) {
        val base = "${RELEASE_DOWNLOADS}v$version/"
        bodies[base + "ngterm-$version-android.apk"] = apkBytes(version)
        bodies[base + "SHA256SUMS"] = "${sha256Hex(apkBytes(version))}  ngterm-$version-android.apk\n".toByteArray()
        bodies[base + "SHA256SUMS.minisig"] = "signed $version".toByteArray()
    }

    override suspend fun fetch(
        url: String,
        limit: Long,
    ): ByteArray {
        fetched += url
        fetchGate?.await()
        failure?.let { throw it }
        return bodies[url] ?: throw IOException("GitHub answered 404")
    }

    override suspend fun download(
        url: String,
        into: File,
        size: Long,
        progress: (Long) -> Unit,
    ) {
        try {
            gate?.await()
            val bytes = bodies[url] ?: throw IOException("GitHub answered 404")
            progress(size / 2)
            stall?.let { latch ->
                stall = null
                latch.await()
            }
            into.writeBytes(bytes)
            progress(size)
        } finally {
            ended.incrementAndGet()
        }
    }
}

/** A checksum check like the core's over [FakeHttp]'s lists: the signature must read `signed <version>`. */
val fakeChecksum: (ByteArray, ByteArray, String, String) -> String = { manifest, signature, version, asset ->
    if (signature.decodeToString() != "signed $version") throw ReleaseException.Signature()
    manifest
        .decodeToString()
        .lines()
        .firstOrNull { it.endsWith("  $asset") }
        ?.substringBefore("  ")
        ?: throw ReleaseException.Checksum(asset)
}

/** [ApkInspector] of an installed [version] (code [code]) and of APKs whose facts the test sets. */
class FakeApks(
    var version: String = "0.2.3",
    var code: Long = 2_003,
) : ApkInspector {
    var archive: ApkFacts? = ApkFacts(APP_PACKAGE, 3_000, setOf(APP_KEY))

    override fun installed() = version to ApkFacts(APP_PACKAGE, code, setOf(APP_KEY))

    override fun archive(apk: File) = archive
}

/** The fakes behind a [SelfUpdates], with what it did to them. */
class UpdateRig(
    root: File,
    store: DataStore<Preferences> = testStore(root, "updates"),
) {
    val http = FakeHttp()
    val apks = FakeApks()
    val prefs = UpdatePrefs(store)
    val installed = CopyOnWriteArrayList<File>()
    val announced = CopyOnWriteArrayList<String>()

    @Volatile var cleared = 0
    val schedule = CopyOnWriteArrayList<Boolean>()
    val launched = CopyOnWriteArrayList<Intent>()
    var installFailure: Exception? = null
    val downloads = File(root, "downloads")

    val parts =
        UpdateParts(
            prefs = prefs,
            http = http,
            apks = apks,
            verifier = UpdateVerifier(fakeChecksum, apks),
            installer = { apk ->
                installFailure?.let { throw it }
                installed += apk
            },
            notices =
                object : UpdateNotices {
                    override fun available(offer: UpdateOffer) {
                        announced += offer.version
                    }

                    override fun clear() {
                        cleared++
                    }
                },
            schedule =
                object : UpdateSchedule {
                    override fun start() {
                        schedule += true
                    }

                    override fun stop() {
                        schedule += false
                    }
                },
            downloads = downloads,
            launch = { launched += it },
        )

    /** What escaped the updater's coroutines, which on the app's scope would crash the app. */
    val uncaught = CopyOnWriteArrayList<Throwable>()

    val scope =
        CoroutineScope(
            SupervisorJob() + Dispatchers.Default.limitedParallelism(1) +
                CoroutineExceptionHandler { _, error -> uncaught += error },
        )

    fun updates() = SelfUpdates(scope, Dispatchers.IO, parts)
}

/** Waits (a few seconds at most) for [updates] to reach a state [matching]. */
fun awaitState(
    updates: AppUpdates,
    matching: (UpdateState) -> Boolean,
): UpdateState = runBlocking { withTimeout(5_000) { updates.state.first(matching) } }

/** A store whose file can't be read or written, as a corrupt file or a full disk leaves it. */
class BrokenStore : DataStore<Preferences> {
    override val data: Flow<Preferences> = flow { throw CorruptionException("unreadable") }

    override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences =
        throw IOException("no space left on device")
}
