package com.nickalie.ngterm.android

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.KeyMaker
import com.nickalie.ngterm.android.data.MachineStore
import com.nickalie.ngterm.android.data.StoredMachine
import com.nickalie.ngterm.android.platform.KeyWrapper
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.ffi.KeyException
import com.nickalie.ngterm.core.ffi.KeyPair
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import java.io.File
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The public key line the fake key maker hands out: a made-up key, never a real one. */
const val FAKE_PUBLIC_KEY =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeKeyForScreenshotsOnlyNotARealKey0000000000 ngterm@pixel-9"

/** The fake key's fingerprint. */
const val FAKE_FINGERPRINT = "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E"

/** The passphrase that opens [ENCRYPTED_FAKE_KEY]. */
const val FAKE_PASSPHRASE = "correct horse"

/** A "private key" the fake key maker reads as encrypted: text in a key's shape, holding nothing. */
const val ENCRYPTED_FAKE_KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nENCRYPTED fake\n-----END OPENSSH PRIVATE KEY-----"

/** A "private key" the fake key maker reads as plain. */
const val PLAIN_FAKE_KEY = "-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----"

/**
 * The core's key functions, faked: [generate] hands out [FAKE_PUBLIC_KEY]; [import] takes text starting with
 * `-----BEGIN`, wants [FAKE_PASSPHRASE] when it says ENCRYPTED, and refuses anything else as the core would.
 */
class FakeKeyMaker : KeyMaker {
    var generated = 0
        private set

    /** What [import] was last handed, as text (the screen's bytes are wiped once read). */
    var lastImported: String? = null
        private set

    override fun generate(comment: String): KeyPair {
        generated += 1
        return KeyPair("fake private key $generated".toByteArray(), FAKE_PUBLIC_KEY, FAKE_FINGERPRINT)
    }

    override fun import(
        privateKey: ByteArray,
        passphrase: String?,
    ): KeyPair {
        val text = String(privateKey)
        lastImported = text
        if (!text.startsWith("-----BEGIN")) throw KeyException.Invalid("unknown key format")
        if ("ENCRYPTED" in text) {
            if (passphrase == null) throw KeyException.PassphraseRequired()
            if (passphrase != FAKE_PASSPHRASE) throw KeyException.WrongPassphrase()
        }
        return KeyPair(privateKey.copyOf(), FAKE_PUBLIC_KEY.replace("ngterm@pixel-9", "imported"), FAKE_FINGERPRINT)
    }
}

/** AES-GCM under a software key: the Keystore's wrapping, for the JVM. */
class SoftwareWrapper : KeyWrapper {
    private val key: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    override fun wrap(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        return cipher.iv + cipher.doFinal(plain)
    }

    override fun unwrap(sealed: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, sealed, 0, 12))
        return cipher.doFinal(sealed, 12, sealed.size - 12)
    }
}

/** A Preferences DataStore in [dir]/[name]. */
fun testStore(
    dir: File,
    name: String,
): DataStore<Preferences> = PreferenceDataStoreFactory.create { File(dir, "$name.preferences_pb") }

/**
 * The app's graph over fakes in [dir]: the [core] a [FakeMobileCore] pinning through the real [PinnedHostKeys], the
 * stores real DataStores, the vault sealing with a [SoftwareWrapper], the clock [now] and the network [online] set by
 * the test. [scope] runs the connections' calls (the main thread's in UI tests); [close] stops it.
 */
class TestGraph(
    dir: File,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
) {
    val online = MutableStateFlow(true)
    var now = 1_000_000L
    val maker = FakeKeyMaker()
    val vault = KeystoreKeyVault(File(dir, "keys"), SoftwareWrapper())
    val pins = PinnedHostKeys(testStore(dir, "pins"))
    lateinit var core: FakeMobileCore
        private set
    val connections = Connections(scope, { now }) { sink -> FakeMobileCore(sink, pins).also { core = it } }
    val graph =
        AppGraph(
            machines = MachineStore(testStore(dir, "machines")),
            keys = AppKeys(testStore(dir, "keys"), vault, maker, "ngterm@test"),
            pins = pins,
            connections = connections,
            online = online,
        )

    /** Makes the app's first key, as the first run would. */
    fun seedKey(): AppKey = runBlocking { graph.keys.generate() }

    /** Adds [machine] to the store. */
    fun seedMachine(machine: StoredMachine) = runBlocking { graph.machines.save(machine) }

    fun close() = scope.cancel()
}

/** A machine for tests, signing in with [keyId]. */
fun testMachine(
    id: String = "m1",
    label: String = "build-01",
    host: String = "build-01.lan",
    keyId: String = "k1",
) = StoredMachine(id = id, label = label, host = host, port = 22, user = "nick", keyId = keyId)
