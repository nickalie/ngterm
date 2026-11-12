package com.nickalie.ngterm.android.update

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import java.io.IOException

/**
 * The update choices, kept in [store] (Preferences DataStore): whether the app checks on its own (on until turned
 * off), the version the user said Not now to, and the last version a notification announced, so each is announced
 * once. A store that can't be read reads as the defaults, and an edit that can't be written is dropped.
 */
class UpdatePrefs(
    private val store: DataStore<Preferences>,
) {
    private val data: Flow<Preferences> =
        store.data.catch { error -> if (error is IOException) emit(emptyPreferences()) else throw error }

    /** Whether the app checks on its own. */
    val automatic: Flow<Boolean> = data.map { it[AUTOMATIC] ?: true }

    /** The version the user skipped. */
    suspend fun skipped(): String? = data.first()[SKIPPED]

    /** The version a notification announced last. */
    suspend fun announced(): String? = data.first()[ANNOUNCED]

    /** False when the choice couldn't be stored. */
    suspend fun setAutomatic(on: Boolean): Boolean = edit { it[AUTOMATIC] = on }

    suspend fun skip(version: String) {
        edit { it[SKIPPED] = version }
    }

    suspend fun announce(version: String) {
        edit { it[ANNOUNCED] = version }
    }

    private suspend fun edit(change: (MutablePreferences) -> Unit): Boolean =
        try {
            store.edit(change)
            true
        } catch (_: IOException) {
            false
        }

    private companion object {
        val AUTOMATIC = booleanPreferencesKey("automatic")
        val SKIPPED = stringPreferencesKey("skipped")
        val ANNOUNCED = stringPreferencesKey("announced")
    }
}
