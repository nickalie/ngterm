package com.nickalie.ngterm.android

import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.PickRow
import com.nickalie.ngterm.core.ffi.RemotePickItem

/**
 * The `/` menu's ranking for the app's tests, which run without the native library (the core's `filter_commands`):
 * the commands whose name holds [query]'s letters in order, ignoring case, those starting with it first, then A to Z;
 * a blank query lists them all A to Z.
 */
fun rankCommands(
    query: String,
    items: List<AgentCommand>,
): List<AgentCommand> {
    val wanted = query.trim().lowercase()
    val matching = items.filter { it.name.lowercase().holdsInOrder(wanted) }
    return matching.sortedWith(compareBy({ !it.name.lowercase().startsWith(wanted) }, { it.name.lowercase() }))
}

private fun String.holdsInOrder(letters: String): Boolean {
    var at = 0
    for (ch in this) if (at < letters.length && ch == letters[at]) at++
    return at == letters.length
}

/**
 * A stand-in for the core's `pickRows` on the JVM (the native core isn't loaded there): a blank query keeps every item
 * in order, else the items whose label holds the query's letters in order; none, with custom answers allowed, offers
 * the custom row.
 */
fun rankPicks(
    items: List<RemotePickItem>,
    query: String,
    allowCustom: Boolean,
): List<PickRow> {
    val wanted = query.trim()
    val matching =
        if (wanted.isEmpty()) {
            items.indices
        } else {
            items.indices.filter {
                items[it].label.lowercase().holdsInOrder(wanted.lowercase())
            }
        }
    return when {
        wanted.isNotEmpty() && matching.none() && allowCustom -> listOf(PickRow.Custom("Use \"$wanted\"", wanted))
        else -> matching.map { PickRow.Item(it.toUInt()) }
    }
}
