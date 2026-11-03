package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/**
 * The kinds of notification the user can turn off, each on until then. An ask is not among them: it waits for an
 * answer, so it always reaches the user.
 */
enum class AlertKind(
    internal val key: Preferences.Key<Boolean>,
) {
    /** A notification a session's agent sent (`ngtermctl notify`). */
    Notices(booleanPreferencesKey("notify_notices")),

    /** An agent stopped to wait for the user (status `blocked`). */
    Waiting(booleanPreferencesKey("notify_waiting")),

    /** An agent finished (status `completed`). */
    Done(booleanPreferencesKey("notify_done")),

    /** A machine with attached sessions was lost for good (it needs the user to connect it again). */
    Errors(booleanPreferencesKey("notify_errors")),
}

/**
 * The user's notification choices, kept in [store] (Preferences DataStore): which [AlertKind]s are on (Settings
 * switches them), and whether the app has asked for the notification permission already, so it asks once.
 */
class NotificationPrefs(
    private val store: DataStore<Preferences>,
) {
    /** The kinds that are on. */
    val enabled: Flow<Set<AlertKind>> =
        store.data.map { prefs -> AlertKind.entries.filterTo(mutableSetOf()) { prefs[it.key] ?: true } }

    /** Whether the app has asked for the notification permission (or the user said Not now). */
    val permissionAsked: Flow<Boolean> = store.data.map { it[PERMISSION_ASKED] ?: false }

    /** Turns [kind] on or off. */
    suspend fun set(
        kind: AlertKind,
        on: Boolean,
    ) {
        store.edit { it[kind.key] = on }
    }

    /** The permission question has had its answer: it isn't asked again (Settings turns notifications on later). */
    suspend fun markPermissionAsked() {
        store.edit { it[PERMISSION_ASKED] = true }
    }

    private companion object {
        val PERMISSION_ASKED = booleanPreferencesKey("notification_permission_asked")
    }
}
