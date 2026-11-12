package com.nickalie.ngterm.android.update

/** A release's `MAJOR.MINOR.PATCH[-PRERELEASE]` version, ordered as Semantic Versioning orders them. */
data class ReleaseVersion(
    val major: Long,
    val minor: Long,
    val patch: Long,
    val pre: List<String> = emptyList(),
) : Comparable<ReleaseVersion> {
    /** Whether it is a prerelease (`0.3.0-rc.1`). */
    val isPre: Boolean get() = pre.isNotEmpty()

    override fun compareTo(other: ReleaseVersion): Int =
        compareValuesBy(this, other, { it.major }, { it.minor }, { it.patch }).takeIf { it != 0 }
            ?: comparePre(pre, other.pre)

    override fun toString(): String = "$major.$minor.$patch" + if (isPre) "-" + pre.joinToString(".") else ""

    companion object {
        private val SHAPE =
            Regex(
                """(0|[1-9]\d{0,17})\.(0|[1-9]\d{0,17})\.(0|[1-9]\d{0,17})(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?""",
            )

        /** [text] as a version, its `+build` part ignored; `null` when it isn't one. */
        fun parse(text: String): ReleaseVersion? {
            val match = SHAPE.matchEntire(text.substringBefore('+')) ?: return null
            val (major, minor, patch, pre) = match.destructured
            return ReleaseVersion(
                major.toLong(),
                minor.toLong(),
                patch.toLong(),
                pre.split('.').filter { it.isNotEmpty() },
            )
        }

        // a release outranks its prereleases; numeric identifiers rank below others and by value; a longer list wins a tie
        private fun comparePre(
            a: List<String>,
            b: List<String>,
        ): Int {
            if (a.isEmpty() != b.isEmpty()) return if (a.isEmpty()) 1 else -1
            for ((x, y) in a.zip(b)) {
                val order = compareIdentifier(x, y)
                if (order != 0) return order
            }
            return a.size.compareTo(b.size)
        }

        private fun compareIdentifier(
            x: String,
            y: String,
        ): Int {
            val nx = x.toLongOrNull()
            val ny = y.toLongOrNull()
            return when {
                nx != null && ny != null -> nx.compareTo(ny)
                nx != null -> -1
                ny != null -> 1
                else -> x.compareTo(y)
            }
        }
    }
}
