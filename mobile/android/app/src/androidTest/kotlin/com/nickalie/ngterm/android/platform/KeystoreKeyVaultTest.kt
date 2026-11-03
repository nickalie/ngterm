package com.nickalie.ngterm.android.platform

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.core.ffi.VaultException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/** The vault on a real Android Keystore: what it seals opens again, and the file holds no plain bytes. */
@RunWith(AndroidJUnit4::class)
class KeystoreKeyVaultTest {
    private val dir = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "vault-test")

    @Test
    fun aKeySealedByTheKeystoreOpensAgainAndIsNotStoredPlain() {
        dir.deleteRecursively()
        val vault = KeystoreKeyVault(dir, AndroidKeystoreWrapper(alias = "ngterm.vault.test"))
        val secret = "a fake private key for the test".toByteArray()
        vault.store("k1", secret.copyOf())
        val sealed = File(dir, "k1.key").readBytes()
        assertFalse(String(sealed, Charsets.ISO_8859_1).contains("fake private key"))
        assertArrayEquals(secret, vault.load("k1"))
        // a second seal of the same bytes differs: the Keystore picks a fresh IV each time
        vault.store("k2", secret.copyOf())
        assertFalse(sealed.contentEquals(File(dir, "k2.key").readBytes()))
        assertTrue(vault.has("k2"))
        File(dir, "k1.key").writeBytes(sealed.copyOf().also { it[it.size - 1] = (it.last() + 1).toByte() })
        assertThrows(VaultException.Unavailable::class.java) { vault.load("k1") }
        dir.deleteRecursively()
    }
}
