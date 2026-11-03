package com.nickalie.ngterm.android.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.RemoteInput
import com.nickalie.ngterm.android.GraphOwner
import com.nickalie.ngterm.android.data.Asks
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.askTag
import com.nickalie.ngterm.android.ui.asks.askReply
import com.nickalie.ngterm.android.ui.asks.fitAskText
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAsk

/**
 * Answers an ask from its notification: the button an action names, with an input ask's inline reply as its text.
 * The ask leaves the open asks, which takes its notification down; an ask no longer open (answered or taken back
 * meanwhile, or lost with a process the system ended) is left be, and its notification, which nothing else would
 * take down any more, is cancelled.
 */
class AskActionReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val action = askAction(intent) ?: return
        val typed = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_ASK_REPLY)?.toString()
        val owner = context.applicationContext as? GraphOwner ?: return
        if (!answerFromNotification(owner.graph.asks, action, typed)) {
            Notifications(context).cancel(askTag(SessionKey(action.machineId, action.sessionId), action.askId))
        }
    }
}

/**
 * Answers [action]'s ask in [asks] with its button and [typed] (an inline reply, `null` for none); false when the ask
 * is no longer open.
 */
fun answerFromNotification(
    asks: Asks,
    action: AskAction,
    typed: String?,
): Boolean {
    val key = SessionKey(action.machineId, action.sessionId)
    val open = asks.find(key, action.askId) ?: return false
    return asks.answer(key, action.askId, notificationReply(open.ask, action.buttonId, typed))
}

/**
 * The answer of [buttonId] pressed on [ask]'s notification: the cancel button escapes; an input ask answers with the
 * reply [typed] there, fitted to its field (a line field's line breaks become spaces), or its prefill when nothing was
 * typed; anything else as the sheet would with the ask's initial values.
 */
fun notificationReply(
    ask: RemoteAsk,
    buttonId: String,
    typed: String?,
): AskReply {
    val input = ask.input
    if (input == null || buttonId == ask.cancelId) return askReply(ask, buttonId)
    return AskReply(button = buttonId, text = fitAskText(typed ?: input.value, input.multiline), selected = null)
}
