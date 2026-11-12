package com.nickalie.ngterm.android.update

import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.os.Build
import androidx.core.content.pm.PackageInfoCompat
import com.nickalie.ngterm.core.ffi.ReleaseException
import java.io.File
import java.io.InputStream
import java.security.MessageDigest

/** What the update check needs of an APK: its package, version code and its signers' certificate SHA-256s. */
data class ApkFacts(
    val packageName: String,
    val versionCode: Long,
    val signers: Set<String>,
)

/** The installed app's [ApkFacts] and a downloaded APK's. */
interface ApkInspector {
    /** The installed app's facts, its version name among them. */
    fun installed(): Pair<String, ApkFacts>

    /** [apk]'s facts; `null` when it isn't an APK Android can read. */
    fun archive(apk: File): ApkFacts?
}

/** [ApkInspector] over the [PackageManager]. */
class PackageApks(
    context: Context,
) : ApkInspector {
    private val context = context.applicationContext

    @Suppress("DEPRECATION") // GET_SIGNATURES is Android 8's only way to read signers
    private val flags =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            PackageManager.GET_SIGNING_CERTIFICATES
        } else {
            PackageManager.GET_SIGNATURES
        }

    override fun installed(): Pair<String, ApkFacts> {
        val info = context.packageManager.getPackageInfo(context.packageName, flags)
        return info.versionName.orEmpty() to facts(info)
    }

    override fun archive(apk: File): ApkFacts? =
        context.packageManager.getPackageArchiveInfo(apk.path, flags)?.let(::facts)

    private fun facts(info: PackageInfo) =
        ApkFacts(info.packageName, PackageInfoCompat.getLongVersionCode(info), signers(info).map(::certificate).toSet())

    @Suppress("DEPRECATION") // PackageInfo.signatures is Android 8's only way to read signers
    private fun signers(info: PackageInfo): List<Signature> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.signingInfo
                ?.apkContentsSigners
                ?.toList()
                .orEmpty()
        } else {
            info.signatures?.toList().orEmpty()
        }

    private fun certificate(signature: Signature) = sha256(signature.toByteArray().inputStream())
}

/** A downloaded update refused before Android's installer saw it. */
class VerifyException(
    val problem: VerifyProblem,
) : Exception("the update didn't verify: $problem")

/**
 * Checks a downloaded APK before it reaches Android's installer: the release's checksum list is signed with
 * ngterm's release key for this very release ([checksum], the mobile core's check, the one desktop builds make), the
 * APK matches the checksum it lists, and the APK is this app, newer than the installed one and signed with the same
 * key (Android would refuse another key, but only after asking the user).
 */
class UpdateVerifier(
    private val checksum: (manifest: ByteArray, signature: ByteArray, version: String, asset: String) -> String,
    private val apks: ApkInspector,
) {
    /** @throws VerifyException with what failed. */
    fun verify(
        release: ReleaseDownload,
        apk: File,
        manifest: ByteArray,
        signature: ByteArray,
    ) {
        val expected =
            try {
                checksum(manifest, signature, release.version.toString(), release.apk)
            } catch (error: ReleaseException) {
                throw VerifyException(
                    if (error is ReleaseException.Signature) VerifyProblem.Signature else VerifyProblem.Checksum,
                )
            }
        if (apk.inputStream().use(::sha256) != expected) throw VerifyException(VerifyProblem.Checksum)
        val (_, installed) = apks.installed()
        val archive = apks.archive(apk) ?: throw VerifyException(VerifyProblem.Package)
        if (archive.packageName != installed.packageName || archive.versionCode <= installed.versionCode) {
            throw VerifyException(VerifyProblem.Package)
        }
        if (archive.signers.isEmpty() ||
            archive.signers != installed.signers
        ) {
            throw VerifyException(VerifyProblem.Signer)
        }
    }
}

private fun sha256(input: InputStream): String {
    val digest = MessageDigest.getInstance("SHA-256")
    val buffer = ByteArray(64 * 1024)
    while (true) {
        val read = input.read(buffer)
        if (read < 0) break
        digest.update(buffer, 0, read)
    }
    return digest.digest().joinToString("") { "%02x".format(it) }
}
