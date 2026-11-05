package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * Whether panes running an agent show as a chat: Settings' Agent chat view as the [default], and the sessions whose
 * More menu set their own ([sessions], by session).
 */
data class ChatViews(
    val default: Boolean = AppSettings().agentChat,
    val sessions: Map<SessionKey, Boolean> = emptyMap(),
) {
    /** Whether [key]'s panes running an agent show as a chat: its own choice, else the default. */
    fun on(key: SessionKey): Boolean = sessions[key] ?: default
}

/**
 * Whether each session's panes running an agent show as a chat ([views]): Settings' Agent chat view (in [settings]) as
 * the default, and the sessions whose More menu set their own, kept in [store] (Preferences DataStore); a session set
 * back to the default then is dropped. The newest [MAX_SESSIONS] are kept, so sessions ended long ago don't pile up;
 * one dropped that way follows the default again. Each source that can't be read stands in for itself: unreadable
 * settings as the default, an unreadable store (or a value in it) as no choices, so neither stops the other.
 */
class SessionChatViews(
    private val store: DataStore<Preferences>,
    settings: Flow<AppSettings>,
) {
    /** The sessions' own choices, by session; none while the store can't be read. */
    val sessions: Flow<Map<SessionKey, Boolean>> =
        store.data
            .map { decode(it[VIEWS]).toMap() }
            .catch { emit(emptyMap()) }

    /** The default and the sessions' own choices, on every change of either. */
    val views: Flow<ChatViews> =
        combine(
            settings
                .map { it.agentChat }
                .catch { emit(ChatViews().default) }
                .distinctUntilChanged(),
            sessions,
            ::ChatViews,
        )

    /**
     * Shows [key]'s agents as a chat ([on]) or as their terminal; a choice equal to [default] is dropped. Throws
     * [java.io.IOException] when the store can't be read or written.
     */
    suspend fun set(
        key: SessionKey,
        on: Boolean,
        default: Boolean,
    ) {
        store.edit { prefs ->
            val kept = decode(prefs[VIEWS]).filter { it.first != key }
            val now = if (on == default) kept else (kept + (key to on)).takeLast(MAX_SESSIONS)
            prefs[VIEWS] = encode(now)
        }
    }

    internal companion object {
        /** The most sessions kept with a choice of their own. */
        const val MAX_SESSIONS = 256

        private val VIEWS = stringPreferencesKey("session_chat_views")

        // oldest first
        fun decode(text: String?): List<Pair<SessionKey, Boolean>> {
            if (text == null) return emptyList()
            return try {
                val array = JSONArray(text)
                List(array.length()) { index ->
                    val item = array.getJSONObject(index)
                    SessionKey(item.getString("machine"), item.getString("session")) to item.getBoolean("chat")
                }
            } catch (_: JSONException) {
                emptyList()
            }
        }

        fun encode(views: List<Pair<SessionKey, Boolean>>): String =
            JSONArray(
                views.map { (key, on) ->
                    JSONObject().put("machine", key.machineId).put("session", key.sessionId).put("chat", on)
                },
            ).toString()
    }
}
