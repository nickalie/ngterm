package com.nickalie.ngterm.android.e2e

import android.app.Instrumentation
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.rules.ExternalResource

/**
 * The test APK's own keyboards ([QaKeyboard], [QaVoiceTyping]) on for a test, and the device's own input method back
 * after it; the device's animations off meanwhile, so a key is tapped where it rests, not where it slides in from.
 * A test types on them key by key ([tapKeys]), as a person does on a soft keyboard.
 */
internal class QaKeyboards(
    private val instrumentation: Instrumentation,
) : ExternalResource() {
    private val device = UiDevice.getInstance(instrumentation)
    private val testPackage = instrumentation.context.packageName
    private var before: String? = null
    private val scales = mutableMapOf<String, String>()

    /** The input method ids of the test's keyboard and of its voice-typing stand-in. */
    val keyboard: String get() = QaKeyboard.id(testPackage)
    val voiceTyping: String get() = QaVoiceTyping.id(testPackage)

    override fun before() {
        ANIMATION_SCALES.forEach { name ->
            scales[name] = shell("settings get global $name").trim()
            shell("settings put global $name 0")
        }
        before = current().takeIf { it.contains('/') }
        // a test APK installed a moment ago may not be listed as input methods yet: until it is, try again
        val deadline = System.currentTimeMillis() + KEY_WAIT_MS
        while (current() != keyboard && System.currentTimeMillis() < deadline) {
            val answers = listOf(keyboard, voiceTyping).map { shell("ime enable $it") } + shell("ime set $keyboard")
            Log.i(TAG, "keyboards: ${answers.joinToString(" | ") { it.trim() }}")
            if (current() != keyboard) Thread.sleep(RETRY_MS)
        }
        assertTrue("the test keyboard is the device's input method (now ${current()})", current() == keyboard)
    }

    override fun after() {
        before?.let { shell("ime set $it") } ?: shell("ime reset")
        shell("ime disable $voiceTyping")
        shell("ime disable $keyboard")
        scales.forEach { (name, value) ->
            shell(if (value == "null") "settings delete global $name" else "settings put global $name $value")
        }
    }

    /** Taps each character of [keys] on the soft keyboard. */
    fun tapKeys(keys: String) = keys.forEach { tapKey(QaKeyboard.keyDescription(it.toString())) }

    fun tapKey(description: String) {
        val key = device.wait(Until.findObject(By.desc(description)), KEY_WAIT_MS)
        assertNotNull("the key $description on the soft keyboard", key)
        device.waitForIdle()
        key.click()
    }

    fun awaitKey(description: String) {
        assertNotNull(
            "the key $description on the soft keyboard",
            device.wait(Until.findObject(By.desc(description)), KEY_WAIT_MS),
        )
    }

    private fun current() = shell("settings get secure default_input_method").trim()

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"
        const val KEY_WAIT_MS = 10_000L
        const val RETRY_MS = 500L
        val ANIMATION_SCALES = listOf("window_animation_scale", "transition_animation_scale", "animator_duration_scale")
    }
}
