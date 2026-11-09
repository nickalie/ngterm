package com.nickalie.ngterm.android.platform

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.nickalie.ngterm.core.ffi.HostKeyStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext

/**
 * The host keys pinned on first use, one per machine id, in [store] (Preferences DataStore): the core's
 * [HostKeyStore], the one place it learns a machine's pin from. The core calls [pinned] and [pin] on its own thread
 * and waits for them, so they block; the app's own reads and the removal are suspending. Every read runs off the
 * caller's thread: DataStore keeps a lock over a store's first read on the reader's dispatcher, so a reader on the main
 * thread would make a blocking [pinned] wait for that thread, for good when it is the thread [pinned] blocks.
 */
class PinnedHostKeys(
    private val store: DataStore<Preferences>,
) : HostKeyStore {
    /** Every pin, by machine id, on every change. */
    val all: Flow<Map<String, String>> =
        store.data
            .map { prefs ->
                prefs
                    .asMap()
                    .entries
                    .mapNotNull { (key, value) ->
                        key.name
                            .removePrefix(PREFIX)
                            .takeIf { key.name.startsWith(PREFIX) }
                            ?.let { it to value as String }
                    }.toMap()
            }.flowOn(Dispatchers.IO)

    override fun pinned(machineId: String): String? = runBlocking { get(machineId) }

    override fun pin(
        machineId: String,
        fingerprint: String,
    ) = runBlocking { set(machineId, fingerprint) }

    /** The pin of [machineId], if any. */
    suspend fun get(machineId: String): String? = withContext(Dispatchers.IO) { store.data.first()[key(machineId)] }

    /** Pins [fingerprint] for [machineId]. */
    suspend fun set(
        machineId: String,
        fingerprint: String,
    ) {
        store.edit { it[key(machineId)] = fingerprint }
    }

    /**
     * Forgets the pin of [machineId] (Remove pinned key, or the machine removed): its next connect asks about whatever
     * key it offers then.
     */
    suspend fun remove(machineId: String) {
        store.edit { it.remove(key(machineId)) }
    }

    private companion object {
        const val PREFIX = "pin."

        fun key(machineId: String) = stringPreferencesKey(PREFIX + machineId)
    }
}
