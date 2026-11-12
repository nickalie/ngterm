package com.nickalie.ngterm.android.update

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

/**
 * The update choices, kept in [store] (Preferences DataStore): whether the app checks on its own (on until turned
 * off), the version the user said Not now to, and the last version a notification announced, so each is announced
 * once.
 */
class UpdatePrefs(
    private val store: DataStore<Preferences>,
) {
    /** Whether the app checks on its own. */
    val automatic: Flow<Boolean> = store.data.map { it[AUTOMATIC] ?: true }

    /** The version the user skipped. */
    suspend fun skipped(): String? = store.data.first()[SKIPPED]

    /** The version a notification announced last. */
    suspend fun announced(): String? = store.data.first()[ANNOUNCED]

    suspend fun setAutomatic(on: Boolean) {
        store.edit { it[AUTOMATIC] = on }
    }

    suspend fun skip(version: String) {
        store.edit { it[SKIPPED] = version }
    }

    suspend fun announce(version: String) {
        store.edit { it[ANNOUNCED] = version }
    }

    private companion object {
        val AUTOMATIC = booleanPreferencesKey("automatic")
        val SKIPPED = stringPreferencesKey("skipped")
        val ANNOUNCED = stringPreferencesKey("announced")
    }
}
