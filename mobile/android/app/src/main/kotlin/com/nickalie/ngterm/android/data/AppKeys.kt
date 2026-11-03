package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.core.ffi.KeyPair
import com.nickalie.ngterm.core.ffi.generateKey
import com.nickalie.ngterm.core.ffi.importKey
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** What the app shows of one of its keys: never the private half, which only the vault holds. */
data class AppKey(
    val id: String,
    /** The public key as an `authorized_keys` line. */
    val authorizedKey: String,
    /** Its OpenSSH `SHA256:` fingerprint. */
    val fingerprint: String,
    /** Whether the user brought it rather than the app making it. */
    val imported: Boolean,
)

/** Makes and reads ssh keys: the mobile core's [generateKey] and [importKey] in the app, a fake in its tests. */
interface KeyMaker {
    /** A new ed25519 key, its public half commented [comment]. */
    fun generate(comment: String): KeyPair

    /** The key in [privateKey], opened with [passphrase] when it is encrypted. */
    fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ): KeyPair
}

/** The mobile core's key functions. */
object NativeKeyMaker : KeyMaker {
    override fun generate(comment: String): KeyPair = generateKey(comment)

    override fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ): KeyPair = importKey(privateKey, passphrase)
}

/**
 * The app's ssh keys: what it shows of them in [store] (Preferences DataStore), their private halves sealed in
 * [vault]. [generate] and [import] put a new key in both, wiping the private bytes once sealed; they run [maker] off
 * the caller's thread and throw what it throws ([com.nickalie.ngterm.core.ffi.KeyException]).
 */
class AppKeys(
    private val store: DataStore<Preferences>,
    private val vault: KeystoreKeyVault,
    private val maker: KeyMaker,
    private val comment: String,
) {
    /** The keys, oldest first, on every change. */
    val keys: Flow<List<AppKey>> = store.data.map { decode(it[KEYS]) }

    /** The key [id], if the app has it. */
    suspend fun get(id: String): AppKey? = keys.first().firstOrNull { it.id == id }

    /** A new key made on this device. */
    suspend fun generate(): AppKey = add(withContext(Dispatchers.Default) { maker.generate(comment) }, imported = false)

    /** The key the user brought, from [privateKey] (wiped once read) and its [passphrase]. */
    suspend fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ): AppKey {
        val pair =
            try {
                withContext(Dispatchers.Default) { maker.import(privateKey, passphrase?.takeIf { it.isNotEmpty() }) }
            } finally {
                privateKey.fill(0)
            }
        return add(pair, imported = true)
    }

    private suspend fun add(
        pair: KeyPair,
        imported: Boolean,
    ): AppKey {
        val key = AppKey(UUID.randomUUID().toString(), pair.authorizedKey.trim(), pair.fingerprint, imported)
        withContext(Dispatchers.IO) { vault.store(key.id, pair.privateKey) }
        store.edit { prefs -> prefs[KEYS] = encode(decode(prefs[KEYS]) + key) }
        return key
    }

    private companion object {
        val KEYS = stringPreferencesKey("keys")

        fun encode(list: List<AppKey>): String =
            JSONArray(
                list.map {
                    JSONObject()
                        .put("id", it.id)
                        .put("authorizedKey", it.authorizedKey)
                        .put("fingerprint", it.fingerprint)
                        .put("imported", it.imported)
                },
            ).toString()

        fun decode(json: String?): List<AppKey> {
            if (json.isNullOrEmpty()) return emptyList()
            val array = JSONArray(json)
            return (0 until array.length()).map { index ->
                val item = array.getJSONObject(index)
                AppKey(
                    id = item.getString("id"),
                    authorizedKey = item.getString("authorizedKey"),
                    fingerprint = item.getString("fingerprint"),
                    imported = item.optBoolean("imported"),
                )
            }
        }
    }
}
