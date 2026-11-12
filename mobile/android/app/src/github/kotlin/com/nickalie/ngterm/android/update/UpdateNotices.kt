package com.nickalie.ngterm.android.update

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.platform.Notifications

/** Telling the user about an update outside the app. */
interface UpdateNotices {
    /** Announces [offer]; nothing when the app may not post. */
    fun available(offer: UpdateOffer)

    /** Takes the announcement down. */
    fun clear()
}

/** The updates' notification channel. */
const val UPDATES_CHANNEL = "updates"

/** The intent action of the notification's Not now. */
const val ACTION_SKIP_UPDATE = "com.nickalie.ngterm.android.SKIP_UPDATE"

private const val EXTRA_VERSION = "version"
private const val UPDATE_TAG = "update"
private const val UPDATE_ID = 3

/**
 * [UpdateNotices] as one notification on the [UPDATES_CHANNEL]: tapping it opens the update sheet, Install opens it
 * and starts the install, Not now ([SkipUpdateReceiver]) skips the version. Like every notification of the app,
 * nothing posts while the user has notifications off.
 */
class UpdateNotifications(
    context: Context,
) : UpdateNotices {
    private val context = context.applicationContext
    private val manager = NotificationManagerCompat.from(this.context)

    override fun available(offer: UpdateOffer) {
        if (!Notifications(context).allowed()) return
        manager.createNotificationChannel(
            NotificationChannelCompat
                .Builder(UPDATES_CHANNEL, NotificationManagerCompat.IMPORTANCE_DEFAULT)
                .setName(context.getString(R.string.channel_updates))
                .setDescription(context.getString(R.string.channel_updates_body))
                .build(),
        )
        val notification =
            NotificationCompat
                .Builder(context, UPDATES_CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setColor(ContextCompat.getColor(context, R.color.brand_copper))
                .setContentTitle(context.getString(R.string.notify_update_title, offer.version))
                .setContentText(context.getString(R.string.notify_update_body))
                .setContentIntent(activity(ACTION_SHOW_UPDATE))
                .addAction(
                    R.drawable.ic_notification,
                    context.getString(R.string.update_install),
                    activity(ACTION_INSTALL_UPDATE),
                ).addAction(R.drawable.ic_close, context.getString(R.string.update_skip), skip(offer.version))
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .setCategory(NotificationCompat.CATEGORY_RECOMMENDATION)
                .build()
        try {
            manager.notify(UPDATE_TAG, UPDATE_ID, notification)
        } catch (_: SecurityException) {
            // the permission was taken back between the check and the post
        }
    }

    override fun clear() = manager.cancel(UPDATE_TAG, UPDATE_ID)

    private fun activity(action: String): PendingIntent =
        PendingIntent.getActivity(
            context,
            action.hashCode(),
            Intent(context, MainActivity::class.java)
                .setAction(action)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    private fun skip(version: String): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            0,
            Intent(
                context,
                SkipUpdateReceiver::class.java,
            ).setAction(ACTION_SKIP_UPDATE).putExtra(EXTRA_VERSION, version),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
}

/** The update notification's Not now: skips the version it announced. */
class SkipUpdateReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val version = intent.getStringExtra(EXTRA_VERSION)?.takeIf { intent.action == ACTION_SKIP_UPDATE } ?: return
        ((context.applicationContext as? UpdatesOwner)?.updates as? SelfUpdates)?.skip(version)
    }
}
