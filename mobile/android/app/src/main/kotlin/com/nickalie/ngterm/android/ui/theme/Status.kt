package com.nickalie.ngterm.android.ui.theme

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import com.nickalie.ngterm.android.R

/**
 * The glyphs a status is drawn with. The first eleven are upstream's status shapes, in its order, by the name an
 * origin gives them (`circle`, `square`, `triangle-down`, ...: ngterm-core's `StatusShape`); [Ring] and [Slash] are
 * the app's own, for a connection on its way and one out of reach, and have no wire name.
 */
enum class StatusShape(
    @param:DrawableRes val drawable: Int,
    private val wire: String? = null,
) {
    Circle(R.drawable.ic_status_circle, "circle"),
    Square(R.drawable.ic_status_square, "square"),
    Triangle(R.drawable.ic_status_triangle, "triangle"),
    TriangleDown(R.drawable.ic_status_triangle_down, "triangle-down"),
    Play(R.drawable.ic_status_play, "play"),
    Diamond(R.drawable.ic_status_diamond, "diamond"),
    Capsule(R.drawable.ic_status_capsule, "capsule"),
    Seal(R.drawable.ic_status_seal, "seal"),
    Star(R.drawable.ic_status_star, "star"),
    Bolt(R.drawable.ic_status_bolt, "bolt"),
    Bell(R.drawable.ic_status_bell, "bell"),
    Ring(R.drawable.ic_status_ring),
    Slash(R.drawable.ic_status_slash),
    ;

    companion object {
        /** The shape an origin names (its wire spelling), the circle for none or one this build doesn't know. */
        fun fromWire(name: String?): StatusShape =
            name?.let { spelled -> entries.firstOrNull { it.wire.equals(spelled, ignoreCase = true) } } ?: Circle
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

/**
 * An agent's status as its origin names it (ngterm-core's `AgentStatus` wire names), with the word, glyph and color
 * the app gives it when the origin sets none. The origin sends a color and a shape only when the agent overrides them;
 * desktop otherwise falls back to its settings and then its defaults (orange for blocked, green for completed:
 * ngterm-gpui's `status_glyph::resolve`). The app's defaults are its own tones in those roles: working is the healthy
 * verdigris, waiting the attention copper and diamond, done the verdigris star.
 */
enum class AgentState(
    private val wire: String,
    @param:StringRes val word: Int,
    val shape: StatusShape,
) {
    Active("active", R.string.agent_active, StatusShape.Circle),
    Blocked("blocked", R.string.agent_blocked, StatusShape.Diamond),
    Completed("completed", R.string.agent_completed, StatusShape.Star),
    Idle("idle", R.string.status_idle, StatusShape.Capsule),
    ;

    /** This state's color in [colors] when the origin sets none. */
    fun color(colors: StatusColors) =
        when (this) {
            Active, Completed -> colors.online
            Blocked -> colors.attention
            Idle -> colors.idle
        }

    companion object {
        /** The state an origin names (its wire spelling); `null` for none or one this build doesn't know. */
        fun fromWire(name: String?): AgentState? =
            entries.firstOrNull { it.wire.equals(name?.trim(), ignoreCase = true) }
    }
}
