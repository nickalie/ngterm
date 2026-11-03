package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.nickalie.ngterm.core.ffi.Machine
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import org.json.JSONArray
import org.json.JSONObject

/** The default ssh port, which a new machine starts with. */
const val SSH_PORT = 22

/**
 * A machine as the app keeps it: what the user entered and the key it signs in with. Its pinned host key is not here:
 * [com.nickalie.ngterm.android.platform.PinnedHostKeys] holds it, the one place the core reads it from.
 */
data class StoredMachine(
    val id: String,
    val label: String,
    val host: String,
    val port: Int,
    val user: String,
    val keyId: String,
) {
    /** What the list shows for it: its name, or its host when it has none. */
    val title: String get() = label.ifBlank { host }

    /** Its ssh address as `user@host:port`, in the form people compare. */
    val address: String get() = "$user@$host:$port"

    /** The record the core connects, with [pinnedHostKey] as its display copy of the pin. */
    fun toCore(pinnedHostKey: String?): Machine =
        Machine(
            id = id,
            label = title,
            host = host,
            port = port.toUShort(),
            user = user,
            keyId = keyId,
            pinnedHostKey = pinnedHostKey,
        )
}

/**
 * The machines the user added, in the order they were added, kept in [store] (Preferences DataStore, one JSON value).
 * [machines] emits the list on every change and fails, as DataStore does, when the file can't be read.
 */
class MachineStore(
    private val store: DataStore<Preferences>,
) {
    val machines: Flow<List<StoredMachine>> = store.data.map { decode(it[MACHINES]) }

    /** The machine [id], if the user has it. */
    suspend fun get(id: String): StoredMachine? = machines.first().firstOrNull { it.id == id }

    /** Adds [machine], or replaces the one with its id in place. */
    suspend fun save(machine: StoredMachine) {
        store.edit { prefs ->
            val list = decode(prefs[MACHINES]).toMutableList()
            val index = list.indexOfFirst { it.id == machine.id }
            if (index >= 0) list[index] = machine else list += machine
            prefs[MACHINES] = encode(list)
        }
    }

    /** Puts [machine] back at [index] (an undone removal), or at the end when the list has since got shorter. */
    suspend fun restore(
        machine: StoredMachine,
        index: Int,
    ) {
        store.edit { prefs ->
            val list = decode(prefs[MACHINES]).filter { it.id != machine.id }.toMutableList()
            list.add(index.coerceIn(0, list.size), machine)
            prefs[MACHINES] = encode(list)
        }
    }

    /** Removes the machine [id]; nothing happens when there is none. */
    suspend fun delete(id: String) {
        store.edit { prefs -> prefs[MACHINES] = encode(decode(prefs[MACHINES]).filter { it.id != id }) }
    }

    private companion object {
        val MACHINES = stringPreferencesKey("machines")

        fun encode(list: List<StoredMachine>): String =
            JSONArray(
                list.map {
                    JSONObject()
                        .put("id", it.id)
                        .put("label", it.label)
                        .put("host", it.host)
                        .put("port", it.port)
                        .put("user", it.user)
                        .put("keyId", it.keyId)
                },
            ).toString()

        fun decode(json: String?): List<StoredMachine> {
            if (json.isNullOrEmpty()) return emptyList()
            val array = JSONArray(json)
            return (0 until array.length()).map { index ->
                val item = array.getJSONObject(index)
                StoredMachine(
                    id = item.getString("id"),
                    label = item.optString("label"),
                    host = item.getString("host"),
                    port = item.optInt("port", SSH_PORT),
                    user = item.getString("user"),
                    keyId = item.getString("keyId"),
                )
            }
        }
    }
}
