package com.nickalie.ngterm.android.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Where the app was asked to go from outside its screens (a notification's tap): the [pending] target, until the
 * app's frame has gone there and [consume]d it. A later request replaces one not yet taken.
 */
class OpenRequests {
    private val state = MutableStateFlow<OpenTarget?>(null)

    /** The target the app should go to, if any. */
    val pending: StateFlow<OpenTarget?> = state.asStateFlow()

    /** Asks the app to go to [target]. */
    fun request(target: OpenTarget) {
        state.value = target
    }

    /** The app has gone to [target]. */
    fun consume(target: OpenTarget) = state.update { if (it == target) null else it }
}
