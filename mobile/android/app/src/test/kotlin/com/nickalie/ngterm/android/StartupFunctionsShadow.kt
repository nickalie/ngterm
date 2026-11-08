package com.nickalie.ngterm.android

import com.nickalie.ngterm.core.ffi.KeyPair
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements

/** The activity's native version query; all activity, settings and navigation code stays real. */
@Implements(className = "com.nickalie.ngterm.core.ffi.Ngterm_mobileKt", isInAndroidSdk = false)
object StartupFunctionsShadow {
    private const val VERSION = "test-core-version"

    @JvmStatic
    @Implementation
    fun coreVersion(): String = VERSION

    @JvmStatic
    @Implementation
    fun generateKey(comment: String): KeyPair =
        KeyPair("test private bytes".toByteArray(), "ssh-ed25519 test $comment", "SHA256:test")
}
