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
import kotlinx.coroutines.flow.first
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
            store.update {
                it.copy(
                    textSize = 40,
                    font = FontChoice.SystemMono,
                    palette = PaletteChoice.Ghostty,
                    theme = ThemeChoice.Dark,
                    wallpaperColors = true,
                    extraKeys = ExtraKeysChoice.Hidden,
                    keepScreenOn = true,
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
