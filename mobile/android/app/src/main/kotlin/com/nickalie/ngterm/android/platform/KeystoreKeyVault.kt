package com.nickalie.ngterm.android.platform

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.nickalie.ngterm.core.ffi.KeyVault
import com.nickalie.ngterm.core.ffi.VaultException
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Seals and opens the private keys' bytes; the app's is [AndroidKeystoreWrapper]. */
interface KeyWrapper {
    /** [plain] sealed, IV first. */
    fun wrap(plain: ByteArray): ByteArray

    /** What [wrap] sealed into [sealed]. */
    fun unwrap(sealed: ByteArray): ByteArray
}

/**
 * AES-256-GCM under a key that lives in the Android Keystore ([alias], made on first use) and never leaves it: the
 * sealed form is a fresh 12-byte IV the Keystore picks, then the ciphertext with its 16-byte tag.
 */
class AndroidKeystoreWrapper(
    private val alias: String = "ngterm.vault",
) : KeyWrapper {
    private fun secretKey(): SecretKey {
        val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (keyStore.getKey(alias, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec
                .Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(KEY_BITS)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    override fun wrap(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val iv = cipher.iv
        check(iv.size == IV_BYTES) { "the Keystore picked a ${iv.size}-byte IV" }
        return iv + cipher.doFinal(plain)
    }

    override fun unwrap(sealed: ByteArray): ByteArray {
        if (sealed.size <= IV_BYTES) throw GeneralSecurityException("the sealed key is truncated")
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val KEY_BITS = 256
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}

/**
 * The app's private keys, each sealed by [wrapper] in its own file under [dir] (the app's no-backup directory, so a
 * backup never carries one; the Keystore key that opens them wouldn't travel anyway). The core's [KeyVault]: [load]
 * opens one for a connection, which wipes the bytes once used. [store] wipes what it is handed.
 */
class KeystoreKeyVault(
    private val dir: File,
    private val wrapper: KeyWrapper,
) : KeyVault {
    private fun file(keyId: String): File {
        require(ID.matches(keyId)) { "not a key id: $keyId" }
        return File(dir, "$keyId.key")
    }

    /** Seals [privateKey] as [keyId], then wipes it. */
    fun store(
        keyId: String,
        privateKey: ByteArray,
    ) {
        try {
            val sealed = wrapper.wrap(privateKey)
            dir.mkdirs()
            val target = file(keyId)
            val partial = File(dir, "$keyId.partial")
            partial.writeBytes(sealed)
            if (!partial.renameTo(target)) throw IOException("could not store key $keyId")
        } finally {
            privateKey.fill(0)
        }
    }

    /** Whether [keyId] is stored. */
    fun has(keyId: String): Boolean = file(keyId).isFile

    /** Forgets [keyId]. */
    fun delete(keyId: String) {
        file(keyId).delete()
    }

    override fun load(keyId: String): ByteArray {
        val target = runCatching { file(keyId) }.getOrNull()
        if (target == null || !target.isFile) throw VaultException.Missing(keyId)
        return try {
            wrapper.unwrap(target.readBytes())
        } catch (error: GeneralSecurityException) {
            throw VaultException
                .Unavailable("the key could not be unlocked: ${error.javaClass.simpleName}")
                .apply { initCause(error) }
        } catch (error: IOException) {
            throw VaultException.Unavailable("the key could not be read: ${error.message}").apply { initCause(error) }
        }
    }

    private companion object {
        val ID = Regex("[A-Za-z0-9-]+")
    }
}
