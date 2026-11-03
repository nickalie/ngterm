package com.nickalie.ngterm.android.platform

import android.Manifest
import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.Alert
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.AlertSink
import com.nickalie.ngterm.android.data.AskDelivery
import com.nickalie.ngterm.android.data.LostReason
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.core.ffi.RemoteAskButton

/** The one id every tagged notification shares: the tag tells them apart. */
private const val ALERT_ID = 1

/** The foreground service's notification. */
const val LINK_NOTIFICATION_ID = 2

/** The foreground service's channel: quiet, one notification. */
const val LINK_CHANNEL = "links"

/** The intent action that opens [OpenTarget] in [MainActivity]. */
const val ACTION_OPEN = "com.nickalie.ngterm.android.OPEN"

/** The intent action an ask's notification action sends to [AskActionReceiver]. */
const val ACTION_ANSWER_ASK = "com.nickalie.ngterm.android.ANSWER_ASK"

/** The key of an ask's inline reply in the action's results. */
const val KEY_ASK_REPLY = "ask_reply"

private const val EXTRA_MACHINE = "machine"

// the scheme of the data URI that makes each notification's intents distinct
private const val INTENT_SCHEME = "ngterm"
private const val EXTRA_SESSION = "session"
private const val EXTRA_PANE = "pane"
private const val EXTRA_ASK = "ask"
private const val EXTRA_BUTTON = "button"

/** A machine's notification channel: its questions, notifications and agent status. */
fun machineChannel(machineId: String): String = "machine:$machineId"

/** An intent that opens [target] in the app. */
fun openIntent(
    context: Context,
    target: OpenTarget,
): Intent =
    Intent(context, MainActivity::class.java)
        .setAction(ACTION_OPEN)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        .putExtra(EXTRA_MACHINE, target.machineId)
        .putExtra(EXTRA_SESSION, target.sessionId)
        .putExtra(EXTRA_PANE, target.paneId)

/** The target an [openIntent] carries; `null` for any other intent. */
fun openTarget(intent: Intent?): OpenTarget? =
    intent?.takeIf { it.action == ACTION_OPEN }?.let { open ->
        open.getStringExtra(EXTRA_MACHINE)?.let { machine ->
            OpenTarget(machine, open.getStringExtra(EXTRA_SESSION), open.getStringExtra(EXTRA_PANE))
        }
    }

/** What an ask action's intent names: the session, the ask and the button pressed. */
data class AskAction(
    val machineId: String,
    val sessionId: String,
    val askId: String,
    val buttonId: String,
)

/** The [AskAction] of an intent from an ask's notification; `null` for any other intent. */
fun askAction(intent: Intent?): AskAction? {
    val names = listOf(EXTRA_MACHINE, EXTRA_SESSION, EXTRA_ASK, EXTRA_BUTTON)
    val values =
        intent
            ?.takeIf { it.action == ACTION_ANSWER_ASK }
            ?.let { action -> names.mapNotNull(action::getStringExtra) }
            ?.takeIf { it.size == names.size }
    return values?.let { AskAction(machineId = it[0], sessionId = it[1], askId = it[2], buttonId = it[3]) }
}

/**
 * The app's notifications on Android: each [Alert] on its machine's channel (made as it is first needed, named after
 * the machine, so the system's settings can quiet one machine), tapping it opening its session and pane; an ask's
 * buttons as actions or inline replies that [AskActionReceiver] answers; and the foreground service's summary on the
 * quiet [LINK_CHANNEL]. Nothing posts while the user has notifications off (Android 13's permission).
 */
class Notifications(
    context: Context,
) : AlertSink {
    private val context = context.applicationContext
    private val manager = NotificationManagerCompat.from(this.context)

    /** Whether the app may post: the permission is granted (Android 13+) and notifications are on. */
    fun allowed(): Boolean {
        val granted =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED
        return granted && manager.areNotificationsEnabled()
    }

    override fun post(alert: Alert) {
        if (!allowed()) return
        val channel = machineChannel(alert.machineId)
        manager.createNotificationChannel(
            NotificationChannelCompat
                .Builder(channel, NotificationManagerCompat.IMPORTANCE_HIGH)
                .setName(alert.machine)
                .setDescription(context.getString(R.string.channel_machine_body, alert.machine))
                .build(),
        )
        val builder =
            NotificationCompat
                .Builder(context, channel)
                .setSmallIcon(R.drawable.ic_notification)
                .setColor(ContextCompat.getColor(context, R.color.brand_copper))
                .setContentIntent(open(alert.target, alert.tag))
                .setAutoCancel(true)
        describe(builder, alert)
        try {
            manager.notify(alert.tag, ALERT_ID, builder.build())
        } catch (_: SecurityException) {
            // the permission was taken back between the check and the post
        }
    }

    override fun cancel(tag: String) = manager.cancel(tag, ALERT_ID)

    private fun describe(
        builder: NotificationCompat.Builder,
        alert: Alert,
    ) {
        when (alert) {
            is Alert.Notice -> {
                builder
                    .setContentTitle(alert.title)
                    .setContentText(alert.body)
                    .setStyle(NotificationCompat.BigTextStyle().bigText(alert.body))
                    .setSubText(alert.session)
                    .setCategory(NotificationCompat.CATEGORY_EVENT)
            }

            is Alert.Status -> {
                val title =
                    if (alert.kind == AlertKind.Waiting) R.string.notify_waiting_title else R.string.notify_done_title
                builder
                    .setContentTitle(context.getString(title, alert.session))
                    .setContentText(alert.context ?: context.getString(R.string.notify_on_machine, alert.machine))
                    .setCategory(NotificationCompat.CATEGORY_STATUS)
            }

            is Alert.Lost -> {
                val text =
                    when (val reason = alert.reason) {
                        is LostReason.Unreachable -> reason.reason
                        LostReason.HostKeyChanged -> context.getString(R.string.notify_host_key_changed, alert.machine)
                    }
                builder
                    .setContentTitle(context.getString(R.string.notify_lost_title, alert.machine))
                    .setContentText(text)
                    .setStyle(NotificationCompat.BigTextStyle().bigText(text))
                    .setCategory(NotificationCompat.CATEGORY_ERROR)
            }

            is Alert.Ask -> {
                ask(builder, alert)
            }
        }
    }

    // an ask stays up until it is answered (no auto-cancel on a tap, which only opens the app), and says it once
    private fun ask(
        builder: NotificationCompat.Builder,
        alert: Alert.Ask,
    ) {
        val ask = alert.open.ask
        // one answered in the app says so: as its text, or under its message
        val hint = context.getString(R.string.notify_ask_open).takeIf { alert.delivery == AskDelivery.OpenApp }
        val message = ask.message?.takeIf { it.isNotBlank() }
        val text = message ?: hint
        builder
            .setContentTitle(ask.title)
            .setContentText(text)
            .setSubText(context.getString(R.string.notify_session_on, alert.session, alert.machine))
            .setCategory(NotificationCompat.CATEGORY_REMINDER)
            .setAutoCancel(false)
            .setOnlyAlertOnce(true)
        listOfNotNull(message, hint).takeIf { it.isNotEmpty() }?.let { lines ->
            builder.setStyle(NotificationCompat.BigTextStyle().bigText(lines.joinToString("\n")))
        }
        if (alert.delivery == AskDelivery.OpenApp) return
        for (button in ask.buttons) {
            val replies = alert.delivery == AskDelivery.Reply && button.id != ask.cancelId
            builder.addAction(askAction(alert, button, replies))
        }
    }

    private fun askAction(
        alert: Alert.Ask,
        button: RemoteAskButton,
        replies: Boolean,
    ): NotificationCompat.Action {
        val open = alert.open
        val intent =
            Intent(context, AskActionReceiver::class.java)
                .setAction(ACTION_ANSWER_ASK)
                // the data tells the pending intents apart: extras don't, and the request code is only a hash
                .setData(Uri.fromParts(INTENT_SCHEME, "${alert.tag}:${button.id}", null))
                .putExtra(EXTRA_MACHINE, open.key.machineId)
                .putExtra(EXTRA_SESSION, open.key.sessionId)
                .putExtra(EXTRA_ASK, open.ask.id)
                .putExtra(EXTRA_BUTTON, button.id)
        // an inline reply's intent is filled in with the typed text, so it must be mutable
        val mutability = if (replies) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE
        val pending =
            PendingIntent.getBroadcast(
                context,
                "${alert.tag}:${button.id}".hashCode(),
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or mutability,
            )
        val action = NotificationCompat.Action.Builder(R.drawable.ic_notification, button.label, pending)
        if (replies) {
            val hint = open.ask.input?.placeholder ?: context.getString(R.string.ask_answer_hint)
            action
                .addRemoteInput(RemoteInput.Builder(KEY_ASK_REPLY).setLabel(hint).build())
                .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_REPLY)
                .setAllowGeneratedReplies(false)
        }
        return action.build()
    }

    private fun open(
        target: OpenTarget,
        tag: String,
    ): PendingIntent =
        PendingIntent.getActivity(
            context,
            tag.hashCode(),
            openIntent(context, target).setData(Uri.fromParts(INTENT_SCHEME, tag, null)),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    /**
     * The foreground service's notification: how many sessions are attached and which ([sessions], their names), with
     * Detach all; tapping it opens the app.
     */
    fun link(sessions: List<String>): Notification {
        manager.createNotificationChannel(
            NotificationChannelCompat
                .Builder(LINK_CHANNEL, NotificationManagerCompat.IMPORTANCE_LOW)
                .setName(context.getString(R.string.channel_links))
                .setDescription(context.getString(R.string.channel_links_body))
                .setShowBadge(false)
                .build(),
        )
        val detach =
            PendingIntent.getService(
                context,
                0,
                Intent(context, LinkService::class.java).setAction(LinkService.ACTION_DETACH_ALL),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val launch =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val count = sessions.size
        return NotificationCompat
            .Builder(context, LINK_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ContextCompat.getColor(context, R.color.brand_copper))
            .setContentTitle(context.resources.getQuantityString(R.plurals.link_title, count, count))
            .setContentText(sessions.joinToString(", "))
            .setContentIntent(launch)
            .addAction(R.drawable.ic_close, context.getString(R.string.link_detach_all), detach)
            .setOngoing(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }
}
