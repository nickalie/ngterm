package com.nickalie.ngterm.android.update

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import androidx.core.content.IntentCompat
import java.io.File

/** Hands a verified APK to Android's installer. */
fun interface ApkInstaller {
    /** Starts installing [apk]; [InstallResultReceiver] hears how it went. */
    fun install(apk: File)
}

/** What Android's installer said of an install. */
sealed interface InstallResult {
    /** It needs the user to confirm, in [confirm]. */
    data class NeedsYou(
        val confirm: Intent,
    ) : InstallResult

    /** The user backed out. */
    data object Aborted : InstallResult

    /** It failed, with Android's [message]. */
    data class Failed(
        val message: String?,
    ) : InstallResult

    /** It is installed (the app is usually replaced before this arrives). */
    data object Done : InstallResult
}

/** [ApkInstaller] through a [PackageInstaller] session, its status sent to [InstallResultReceiver]. */
class SessionInstaller(
    context: Context,
) : ApkInstaller {
    private val context = context.applicationContext

    override fun install(apk: File) {
        val installer = context.packageManager.packageInstaller
        val params =
            PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
                setAppPackageName(context.packageName)
                setSize(apk.length())
            }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            apk.inputStream().use { input ->
                session.openWrite("base.apk", 0, apk.length()).use { output ->
                    input.copyTo(output)
                    session.fsync(output)
                }
            }
            // the installer fills the status into the intent, so it must stay mutable
            val flags =
                PendingIntent.FLAG_UPDATE_CURRENT or
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
            val status =
                PendingIntent.getBroadcast(
                    context,
                    id,
                    Intent(context, InstallResultReceiver::class.java).setAction(ACTION_INSTALL_STATUS),
                    flags,
                )
            session.commit(status.intentSender)
        }
    }
}

/** The action of the installer's status broadcasts. */
const val ACTION_INSTALL_STATUS = "com.nickalie.ngterm.android.INSTALL_STATUS"

/** The [InstallResult] a status broadcast from Android's installer carries; `null` for any other intent. */
fun installResult(intent: Intent?): InstallResult? {
    if (intent?.action != ACTION_INSTALL_STATUS) return null
    val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
    return when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
        PackageInstaller.STATUS_PENDING_USER_ACTION -> {
            IntentCompat
                .getParcelableExtra(
                    intent,
                    Intent.EXTRA_INTENT,
                    Intent::class.java,
                )?.let(InstallResult::NeedsYou)
                ?: InstallResult.Failed(message)
        }

        PackageInstaller.STATUS_SUCCESS -> {
            InstallResult.Done
        }

        PackageInstaller.STATUS_FAILURE_ABORTED -> {
            InstallResult.Aborted
        }

        else -> {
            InstallResult.Failed(message)
        }
    }
}

/** Hears Android's installer about an update and tells the app's [SelfUpdates]. */
class InstallResultReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val result = installResult(intent) ?: return
        ((context.applicationContext as? UpdatesOwner)?.updates as? SelfUpdates)?.installResult(result)
    }
}
