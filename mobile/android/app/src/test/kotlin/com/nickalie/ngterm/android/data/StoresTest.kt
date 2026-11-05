package com.nickalie.ngterm.android.data

import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.FakeKeyMaker
import com.nickalie.ngterm.android.PLAIN_FAKE_KEY
import com.nickalie.ngterm.android.SoftwareWrapper
import com.nickalie.ngterm.android.platform.KeystoreKeyVault
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.core.ffi.VaultException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException

/** The app's stores over real DataStores (org.json comes from Robolectric's Android). */
@RunWith(AndroidJUnit4::class)
class StoresTest {
    @get:Rule
    val dir = TemporaryFolder()

    @Test
    fun machinesKeepTheirOrderThroughEditsRemovalsAndARestore() =
        runBlocking {
            val store = MachineStore(testStore(dir.root, "machines"))
            val a = testMachine(id = "a", label = "a")
            val b = testMachine(id = "b", label = "b", host = "b.lan")
            val c = testMachine(id = "c", label = "c")
            listOf(a, b, c).forEach { store.save(it) }
            store.save(b.copy(port = 2222))
            assertEquals(listOf(a, b.copy(port = 2222), c), store.machines.first())
            store.delete("b")
            assertEquals(listOf(a, c), store.machines.first())
            store.restore(b, 1)
            assertEquals(listOf(a, b, c), store.machines.first())
            assertEquals(c, store.get("c"))
            assertNull(store.get("nope"))
        }

    @Test
    fun settingsStartAtTheirDefaultsAndKeepWhatIsSet() =
        runBlocking {
            val store = SettingsStore(testStore(dir.root, "settings"))
            assertEquals(AppSettings(), store.settings.first())
            // the chat view is on until turned off
            assertTrue(store.settings.first().agentChat)
            store.update {
                it.copy(
                    textSize = 40,
                    font = FontChoice.SystemMono,
                    palette = PaletteChoice.Ghostty,
                    theme = ThemeChoice.Dark,
                    wallpaperColors = true,
                    extraKeys = ExtraKeysChoice.Hidden,
                    keepScreenOn = true,
                    agentChat = false,
                )
            }
            val again = store.settings.first()
            // a size past the terminal's range is kept at its edge
            assertEquals(32, again.textSize)
            assertEquals(FontChoice.SystemMono, again.font)
            assertEquals(PaletteChoice.Ghostty, again.palette)
            assertEquals(ThemeChoice.Dark, again.theme)
            assertTrue(again.wallpaperColors)
            assertEquals(ExtraKeysChoice.Hidden, again.extraKeys)
            assertTrue(again.keepScreenOn)
            assertFalse(again.agentChat)
        }

    @Test
    fun aSessionKeepsItsChatViewOnlyWhileItDiffersFromTheDefault() =
        runBlocking {
            val store = SessionChatViews(testStore(dir.root, "session_chat_views"), flowOf(AppSettings()))
            val a = SessionKey("m1", "s1")
            val b = SessionKey("m1", "s2")
            assertEquals(emptyMap<SessionKey, Boolean>(), store.sessions.first())

            store.set(a, on = false, default = true)
            store.set(b, on = true, default = false)
            assertEquals(mapOf(a to false, b to true), store.sessions.first())
            assertEquals(false, ChatViews(default = true, sessions = store.sessions.first()).on(a))
            assertEquals(true, ChatViews(default = true, sessions = store.sessions.first()).on(SessionKey("m2", "s1")))

            // set back to the default: dropped
            store.set(a, on = true, default = true)
            assertEquals(mapOf(b to true), store.sessions.first())
        }

    @Test
    fun sessionChatViewsKeepOnlyTheNewest() =
        runBlocking {
            val store = SessionChatViews(testStore(dir.root, "session_chat_views"), flowOf(AppSettings()))
            val keys = List(SessionChatViews.MAX_SESSIONS + 2) { SessionKey("m1", "s$it") }
            keys.forEach { store.set(it, on = false, default = true) }
            // the oldest kept, set again, moves to the newest end: the next new session drops the one after it
            store.set(keys[2], on = false, default = true)
            store.set(SessionKey("m1", "new"), on = false, default = true)
            val kept = store.sessions.first().keys
            assertEquals(SessionChatViews.MAX_SESSIONS, kept.size)
            assertTrue(keys[2] in kept)
            assertTrue(keys[0] !in kept && keys[1] !in kept && keys[3] !in kept)
            assertTrue(keys.last() in kept && keys[4] in kept)
        }

    @Test
    fun unreadableSessionChatViewsReadAsNone() =
        runBlocking {
            val prefs = testStore(dir.root, "session_chat_views")
            prefs.edit { it[stringPreferencesKey("session_chat_views")] = "not json" }
            assertEquals(
                emptyMap<SessionKey, Boolean>(),
                SessionChatViews(prefs, flowOf(AppSettings())).sessions.first(),
            )
        }

    @Test
    fun anUnreadableSessionChatViewsStoreLeavesTheSettingInCharge() =
        runBlocking<Unit> {
            File(dir.root, "broken.preferences_pb").writeBytes(byteArrayOf(0x7f, 0x01, 0x02, 0x03))
            val settings = MutableStateFlow(AppSettings(agentChat = false))
            val store = SessionChatViews(testStore(dir.root, "broken"), settings)
            assertEquals(ChatViews(default = false), store.views.first())
            // the setting still moves the default; a write to the store says it failed
            settings.value = AppSettings(agentChat = true)
            assertEquals(ChatViews(default = true), store.views.first { it.default })
            assertThrows(IOException::class.java) {
                runBlocking { store.set(SessionKey("m1", "s1"), on = false, default = true) }
            }
        }

    @Test
    fun aSettingTheAppNoLongerKnowsReadsAsItsDefault() =
        runBlocking {
            val prefs = testStore(dir.root, "settings")
            prefs.edit {
                it[stringPreferencesKey("palette")] = "Solarized"
                it[intPreferencesKey("text_size")] = 2
            }
            val read = SettingsStore(prefs).settings.first()
            assertEquals(PaletteChoice.Theme, read.palette)
            assertEquals(8, read.textSize)
        }

    @Test
    fun everyMachineCanBePointedAtOneKey() =
        runBlocking {
            val store = MachineStore(testStore(dir.root, "machines"))
            store.save(testMachine(id = "a", keyId = "old"))
            store.save(testMachine(id = "b", keyId = "other"))
            store.useKey("new")
            assertEquals(listOf("new", "new"), store.machines.first().map { it.keyId })
            assertEquals(listOf("a", "b"), store.machines.first().map { it.id })
        }

    @Test
    fun aPinIsReadByTheCoreAndRemovedForGood() =
        runBlocking {
            val pins = PinnedHostKeys(testStore(dir.root, "pins"))
            assertNull(pins.pinned("m1"))
            pins.pin("m1", "SHA256:abc")
            pins.pin("m2", "SHA256:def")
            assertEquals("SHA256:abc", pins.pinned("m1"))
            assertEquals(mapOf("m1" to "SHA256:abc", "m2" to "SHA256:def"), pins.all.first())
            pins.remove("m1")
            assertNull(pins.pinned("m1"))
            assertEquals(mapOf("m2" to "SHA256:def"), pins.all.first())
        }

    @Test
    fun theVaultSealsWhatItStoresAndWipesWhatItIsHanded() {
        val keys = File(dir.root, "keys")
        val vault = KeystoreKeyVault(keys, SoftwareWrapper())
        val secret = "fake private key".toByteArray()
        vault.store("k1", secret)
        assertTrue(secret.all { it == 0.toByte() })
        val sealed = File(keys, "k1.key").readBytes()
        assertFalse(String(sealed).contains("fake private key"))
        assertArrayEquals("fake private key".toByteArray(), vault.load("k1"))
        assertTrue(vault.has("k1"))
        vault.delete("k1")
        assertThrows(VaultException.Missing::class.java) { vault.load("k1") }
    }

    @Test
    fun aKeyThatWontOpenIsUnavailableNotMissing() {
        val keys = File(dir.root, "keys")
        KeystoreKeyVault(keys, SoftwareWrapper()).store("k1", "fake".toByteArray())
        // another wrapper's key: the Keystore key was lost (a restore on another device, say)
        val other = KeystoreKeyVault(keys, SoftwareWrapper())
        assertThrows(VaultException.Unavailable::class.java) { other.load("k1") }
        assertThrows(VaultException.Missing::class.java) { other.load("../k1") }
    }

    @Test
    fun keysAreKeptWithTheirPublicHalfAndTheirPrivateHalfSealed() =
        runBlocking {
            val vault = KeystoreKeyVault(File(dir.root, "keys"), SoftwareWrapper())
            val maker = FakeKeyMaker()
            val keys = AppKeys(testStore(dir.root, "keys"), vault, maker, "ngterm@test")
            val made = keys.generate()
            val bytes = PLAIN_FAKE_KEY.toByteArray()
            val imported = keys.import(bytes, passphrase = "")
            assertTrue("the import wipes what it read", bytes.all { it == 0.toByte() })
            assertEquals(listOf(made, imported), keys.keys.first())
            assertFalse(made.imported)
            assertTrue(imported.imported)
            assertEquals("fake private key 1", String(vault.load(made.id)))
            assertEquals(PLAIN_FAKE_KEY, String(vault.load(imported.id)))
            assertEquals(made, keys.get(made.id))
        }
}
