package com.nickalie.ngterm.android.ui.updates

import android.content.Intent
import com.nickalie.ngterm.android.update.AppUpdates
import com.nickalie.ngterm.android.update.UpdateState
import kotlinx.coroutines.flow.MutableStateFlow

/** [AppUpdates] whose state the test sets, recording what the app asked of it by name. */
class FakeUpdates(
    override val installed: String = "0.2.3",
    initial: UpdateState = UpdateState.Idle,
) : AppUpdates {
    override val state = MutableStateFlow(initial)
    override val automatic = MutableStateFlow(true)
    override val prompt = MutableStateFlow(false)

    /** The calls, oldest first: `check`, `install`, `opened:<action>`... */
    val calls = mutableListOf<String>()

    override fun appStarted() {
        calls += "appStarted"
    }

    override fun setAutomatic(on: Boolean) {
        calls += "automatic:$on"
        automatic.value = on
    }

    override fun check() {
        calls += "check"
    }

    override fun install() {
        calls += "install"
    }

    override fun cancel() {
        calls += "cancel"
    }

    override fun skip() {
        calls += "skip"
    }

    override fun opened(intent: Intent?) {
        calls += "opened:${intent?.action}"
    }

    override fun promptClosed() {
        calls += "promptClosed"
        prompt.value = false
    }
}
