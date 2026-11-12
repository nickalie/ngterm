package com.nickalie.ngterm.android.update

import android.app.Application
import android.content.ActivityNotFoundException
import android.content.Intent
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import com.nickalie.ngterm.core.ffi.releaseChecksum
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import java.io.File

/** The GitHub build's updates, from GitHub's releases. */
fun appUpdates(app: Application): AppUpdates? {
    val apks = PackageApks(app)
    val parts =
        UpdateParts(
            prefs = UpdatePrefs(PreferenceDataStoreFactory.create { app.preferencesDataStoreFile("updates") }),
            http = UrlConnectionHttp("ngterm-android/${apks.installed().first}"),
            apks = apks,
            verifier = UpdateVerifier(::releaseChecksum, apks),
            installer = SessionInstaller(app),
            notices = UpdateNotifications(app),
            schedule = WorkSchedule(app),
            downloads = File(app.cacheDir, "updates"),
            launch = { confirm ->
                try {
                    app.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } catch (_: ActivityNotFoundException) {
                    // no installer to confirm with: the state still offers it again
                }
            },
        )
    return SelfUpdates(CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate), Dispatchers.IO, parts)
}
