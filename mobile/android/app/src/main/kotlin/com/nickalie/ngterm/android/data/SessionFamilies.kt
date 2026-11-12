package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.RemoteSession

/**
 * [listed] by family: each session whose parent isn't listed (the core lists parents first, companions after them
 * all) heads one, in the listing's order, followed by its companions and theirs.
 */
fun families(listed: List<RemoteSession>): List<List<RemoteSession>> {
    val ids = listed.mapTo(mutableSetOf()) { it.id }
    val companions = listed.filter { it.parent in ids }.groupBy { it.parent }
    val placed = mutableSetOf<String>()

    fun family(head: RemoteSession): List<RemoteSession> =
        if (placed.add(head.id)) listOf(head) + companions[head.id].orEmpty().flatMap(::family) else emptyList()

    val heads = listed.filter { it.parent !in ids }.map(::family)
    // a loop of parents has no head: its sessions stand alone, so none goes missing
    val strays = listed.filter { it.id !in placed }.map { listOf(it) }
    return heads + strays
}
