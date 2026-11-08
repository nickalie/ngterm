package com.nickalie.ngterm.android.platform

import android.security.keystore.KeyGenParameterSpec
import java.io.InputStream
import java.io.OutputStream
import java.security.Key
import java.security.KeyStoreSpi
import java.security.Provider
import java.security.SecureRandom
import java.security.Security
import java.security.cert.Certificate
import java.security.spec.AlgorithmParameterSpec
import java.util.Collections
import java.util.Date
import javax.crypto.KeyGenerator
import javax.crypto.KeyGeneratorSpi
import javax.crypto.SecretKey

/** JCA's Android Keystore boundary on the JVM: actual AES/GCM, with software storage instead of device hardware. */
class TestAndroidKeystore : Provider("AndroidKeyStore", 1.0, "Test device key storage") {
    init {
        put("KeyStore.AndroidKeyStore", DeviceKeys::class.java.name)
        put("KeyGenerator.AES", DeviceKeyGenerator::class.java.name)
    }

    companion object {
        val keys = mutableMapOf<String, SecretKey>()
        val specifications = mutableListOf<KeyGenParameterSpec>()

        fun install() {
            keys.clear()
            specifications.clear()
            Security.addProvider(TestAndroidKeystore())
        }

        fun remove() {
            Security.removeProvider("AndroidKeyStore")
            keys.clear()
            specifications.clear()
        }
    }
}

/** An in-memory device store; only the JCA boundary is substituted. */
class DeviceKeys : KeyStoreSpi() {
    override fun engineGetKey(
        alias: String,
        password: CharArray?,
    ): Key? = TestAndroidKeystore.keys[alias]

    override fun engineContainsAlias(alias: String): Boolean = alias in TestAndroidKeystore.keys

    override fun engineAliases() = Collections.enumeration(TestAndroidKeystore.keys.keys)

    override fun engineSize(): Int = TestAndroidKeystore.keys.size

    override fun engineDeleteEntry(alias: String) {
        TestAndroidKeystore.keys.remove(alias)
    }

    override fun engineGetCreationDate(alias: String): Date? = null

    override fun engineGetCertificate(alias: String): Certificate? = null

    override fun engineGetCertificateChain(alias: String): Array<Certificate>? = null

    override fun engineGetCertificateAlias(cert: Certificate): String? = null

    override fun engineIsKeyEntry(alias: String): Boolean = engineContainsAlias(alias)

    override fun engineIsCertificateEntry(alias: String): Boolean = false

    override fun engineLoad(
        stream: InputStream?,
        password: CharArray?,
    ) = Unit

    override fun engineStore(
        stream: OutputStream?,
        password: CharArray?,
    ) = Unit

    override fun engineSetCertificateEntry(
        alias: String,
        cert: Certificate,
    ) = error("not a certificate store")

    override fun engineSetKeyEntry(
        alias: String,
        key: ByteArray,
        chain: Array<Certificate>?,
    ) = error("use the generator")

    override fun engineSetKeyEntry(
        alias: String,
        key: Key,
        password: CharArray?,
        chain: Array<Certificate>?,
    ) = error("use the generator")
}

/** Honors the alias and size requested by Android's actual KeyGenParameterSpec. */
class DeviceKeyGenerator : KeyGeneratorSpi() {
    private lateinit var specification: KeyGenParameterSpec

    override fun engineInit(random: SecureRandom?) = error("a device key needs a specification")

    override fun engineInit(
        keysize: Int,
        random: SecureRandom?,
    ) = error("a device key needs an alias")

    override fun engineInit(
        params: AlgorithmParameterSpec,
        random: SecureRandom?,
    ) {
        specification = params as KeyGenParameterSpec
    }

    override fun engineGenerateKey(): SecretKey =
        KeyGenerator.getInstance("AES", "SunJCE").apply { init(specification.keySize) }.generateKey().also {
            TestAndroidKeystore.keys[specification.keystoreAlias] = it
            TestAndroidKeystore.specifications += specification
        }
}
