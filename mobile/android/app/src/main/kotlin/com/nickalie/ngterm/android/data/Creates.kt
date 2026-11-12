package com.nickalie.ngterm.android.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** A session create: a New session on [machineId], or a Duplicate of its session [source]. */
data class Create(
    val machineId: String,
    val source: String? = null,
)

/**
 * The session create [underWay] on this device, whichever screen began it: one at a time, so two landings never race
 * to open their session.
 */
class Creates {
    private val current = MutableStateFlow<Create?>(null)

    val underWay: StateFlow<Create?> = current.asStateFlow()

    /** Begins [create], or nothing (false) while another is under way. */
    fun begin(create: Create): Boolean = current.compareAndSet(null, create)

    fun end(create: Create) {
        current.compareAndSet(create, null)
    }
}
