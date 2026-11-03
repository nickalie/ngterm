package com.nickalie.ngterm.android.ui.theme

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import com.nickalie.ngterm.android.R

/**
 * The glyphs a status is drawn with. The first six are upstream's status shapes, as an origin names them (`circle`,
 * `square`, ...: ngterm-core's `StatusShape`); [Ring] and [Slash] are the app's own, for a connection on its way and
 * one out of reach.
 */
enum class StatusShape(
    @param:DrawableRes val drawable: Int,
) {
    Circle(R.drawable.ic_status_circle),
    Square(R.drawable.ic_status_square),
    Triangle(R.drawable.ic_status_triangle),
    Diamond(R.drawable.ic_status_diamond),
    Capsule(R.drawable.ic_status_capsule),
    Star(R.drawable.ic_status_star),
    Ring(R.drawable.ic_status_ring),
    Slash(R.drawable.ic_status_slash),
    ;

    companion object {
        private val wire = listOf(Circle, Square, Triangle, Diamond, Capsule, Star)

        /** The shape an origin names (its wire spelling), the circle for none or one this build doesn't know. */
        fun fromWire(name: String?): StatusShape =
            wire.firstOrNull { it.name.equals(name, ignoreCase = true) } ?: Circle
    }
}

/**
 * The app's own statuses: a machine's connection and what an agent asks of you. Each has a color, a glyph and a word,
 * and the three always go out together, so no status rests on color alone.
 */
enum class StatusTone(
    val shape: StatusShape,
    @param:StringRes val label: Int,
) {
    /** Connected and well. */
    Online(StatusShape.Circle, R.string.status_online),

    /** Connecting or reconnecting. */
    Connecting(StatusShape.Ring, R.string.status_connecting),

    /** Not connected, and not trying. */
    Offline(StatusShape.Slash, R.string.status_offline),

    /** Waiting on you: an ask, a host key to confirm. */
    Attention(StatusShape.Diamond, R.string.status_attention),

    /** Failed: a refused key, an unreachable host. */
    Error(StatusShape.Triangle, R.string.status_error),

    /** Nothing happening: an agent with no status. */
    Idle(StatusShape.Capsule, R.string.status_idle),
    ;

    /** This tone's color in [colors]. */
    fun color(colors: StatusColors) =
        when (this) {
            Online -> colors.online
            Connecting -> colors.connecting
            Offline -> colors.offline
            Attention -> colors.attention
            Error -> colors.error
            Idle -> colors.idle
        }
}
