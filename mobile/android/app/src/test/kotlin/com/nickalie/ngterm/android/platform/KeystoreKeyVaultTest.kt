package com.nickalie.ngterm.android.platform

import android.security.keystore.KeyProperties
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.SoftwareWrapper
import com.nickalie.ngterm.core.ffi.VaultException
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException

/** Sealed keys survive a new wrapper, reject tampering, and surface storage failures without keeping plain bytes. */
@RunWith(AndroidJUnit4::class)
class KeystoreKeyVaultTest {
    @get:Rule val dir = TemporaryFolder()

    @Before fun setUp() = TestAndroidKeystore.install()

    @After fun tearDown() = TestAndroidKeystore.remove()

    @Test
    fun deviceWrappingUsesFreshIvsAndReusesTheSame256BitAuthenticatedKey() {
        val plain = "test private bytes".toByteArray()
        val wrapper = AndroidKeystoreWrapper()
        val sealed = wrapper.wrap(plain)
        val second = wrapper.wrap(plain)
        assertFalse(sealed.contentEquals(second))
        assertArrayEquals(plain, AndroidKeystoreWrapper().unwrap(sealed))
        assertArrayEquals(plain, wrapper.unwrap(second))
        val spec = TestAndroidKeystore.specifications.single()
        assertEquals("ngterm.vault", spec.keystoreAlias)
        assertEquals(256, spec.keySize)
        assertEquals(KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT, spec.purposes)
        assertArrayEquals(arrayOf("GCM"), spec.blockModes)
        assertArrayEquals(arrayOf("NoPadding"), spec.encryptionPaddings)
        assertTrue(spec.isRandomizedEncryptionRequired)
        sealed[sealed.lastIndex] = (sealed.last().toInt() xor 1).toByte()
        assertThrows(GeneralSecurityException::class.java) { wrapper.unwrap(sealed) }
    }

    @Test
    fun truncatedKeysAreRefusedAndAnEmptyPlainKeyStillRoundTrips() {
        val wrapper = AndroidKeystoreWrapper("other.alias")
        for (length in listOf(0, 1, 12)) {
            assertThrows(GeneralSecurityException::class.java) { wrapper.unwrap(ByteArray(length)) }
        }
        assertArrayEquals(byteArrayOf(), wrapper.unwrap(wrapper.wrap(byteArrayOf())))
        assertEquals(setOf("other.alias"), TestAndroidKeystore.keys.keys)
    }

    @Test
    fun failingToSealOrPublishWipesTheCallersBytes() {
        val refusing =
            object : KeyWrapper {
                override fun wrap(plain: ByteArray): ByteArray = throw GeneralSecurityException("locked")

                override fun unwrap(sealed: ByteArray): ByteArray = error("not reached")
            }
        val plain = "secret".toByteArray()
        assertThrows(GeneralSecurityException::class.java) { KeystoreKeyVault(dir.root, refusing).store("k1", plain) }
        assertTrue(plain.all { it == 0.toByte() })
        val target = File(dir.root, "k1.key").apply { mkdirs() }
        File(target, "occupied").writeText("keeps the directory nonempty")
        val bytes = "secret".toByteArray()
        assertThrows(IOException::class.java) { KeystoreKeyVault(dir.root, SoftwareWrapper()).store("k1", bytes) }
        assertTrue(bytes.all { it == 0.toByte() })
        assertEquals("keeps the directory nonempty", File(target, "occupied").readText())
    }

    @Test
    fun invalidIdsNeverEscapeTheVaultAndKeysCanBeReplacedThenForgotten() {
        val vault = KeystoreKeyVault(dir.root, AndroidKeystoreWrapper())
        val bytes = "secret".toByteArray()
        assertThrows(IllegalArgumentException::class.java) { vault.store("../escape", bytes) }
        assertTrue(bytes.all { it == 0.toByte() })
        assertThrows(IllegalArgumentException::class.java) { vault.has("") }
        assertThrows(VaultException.Missing::class.java) { vault.load("../escape") }
        assertThrows(VaultException.Missing::class.java) { vault.load("absent") }
        vault.store("key-1", "old".toByteArray())
        vault.store("key-1", "new".toByteArray())
        assertArrayEquals("new".toByteArray(), vault.load("key-1"))
        vault.delete("key-1")
        assertFalse(vault.has("key-1"))
    }

    @Test
    fun aReadFailureIsUnavailableAndRetainsItsCause() {
        val wrapper =
            object : KeyWrapper {
                override fun wrap(plain: ByteArray): ByteArray = plain.copyOf()

                override fun unwrap(sealed: ByteArray): ByteArray = throw IOException("device storage unavailable")
            }
        val vault = KeystoreKeyVault(dir.root, wrapper)
        vault.store("k1", "secret".toByteArray())
        val failure = assertThrows(VaultException.Unavailable::class.java) { vault.load("k1") }
        assertTrue(failure.message.orEmpty().contains("the key could not be read: device storage unavailable"))
        assertTrue(failure.cause is IOException)
    }
}
