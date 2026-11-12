package com.nickalie.ngterm.android.update

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/** The background checks, on or off. */
interface UpdateSchedule {
    fun start()

    fun stop()
}

/** The unique name of the background check's work. */
const val UPDATE_WORK = "update-check"

/** How often the background check runs, in hours. */
const val CHECK_HOURS = 12L

/**
 * [UpdateSchedule] through WorkManager: a check every [CHECK_HOURS] hours on a network connection while the battery
 * isn't low; starting it again keeps the running schedule.
 */
class WorkSchedule(
    context: Context,
) : UpdateSchedule {
    private val context = context.applicationContext

    override fun start() {
        val constraints =
            Constraints
                .Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .setRequiresBatteryNotLow(true)
                .build()
        val work =
            PeriodicWorkRequestBuilder<UpdateCheckWorker>(CHECK_HOURS, TimeUnit.HOURS)
                .setConstraints(constraints)
                .build()
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(UPDATE_WORK, ExistingPeriodicWorkPolicy.KEEP, work)
    }

    override fun stop() {
        WorkManager.getInstance(context).cancelUniqueWork(UPDATE_WORK)
    }
}

/** The background check: announces a new release; a check that couldn't reach GitHub is retried later. */
class UpdateCheckWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val updates = (applicationContext as? UpdatesOwner)?.updates as? SelfUpdates ?: return Result.success()
        return if (updates.checkInBackground()) Result.success() else Result.retry()
    }
}
