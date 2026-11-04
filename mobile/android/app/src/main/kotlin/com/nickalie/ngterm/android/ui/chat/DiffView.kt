package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/** What a line of a unified diff is, for its color. */
enum class DiffLineKind {
    /** A file's header: `diff --git`, `index`, `---`, `+++`. */
    File,

    /** A hunk's `@@ … @@` line. */
    Hunk,
    Added,
    Removed,

    /** A line the change keeps, or anything else. */
    Context,
}

/** One line of a unified diff, as it reads, with what it is. */
data class DiffLine(
    val kind: DiffLineKind,
    val text: String,
)

/** A unified diff's first [lines], and how many more it has ([more]). */
data class DiffLines(
    val lines: List<DiffLine>,
    val more: Int,
)

/** The most lines a diff shows: the rest is the terminal's to show. */
const val DIFF_LINE_LIMIT = 400

/**
 * [diff]'s lines, each with what it is: a file's header lines until its first hunk (a `---` or `+++` inside a hunk is
 * a removed or added line), then the hunks' lines by their first character. At most [limit] lines.
 */
fun diffLines(
    diff: String,
    limit: Int = DIFF_LINE_LIMIT,
): DiffLines {
    val all = diff.trimEnd('\n').lines()
    var header = true
    val lines =
        all.take(limit).map { line ->
            if (line.startsWith("diff ")) header = true
            if (line.startsWith("@@")) header = false
            DiffLine(kind(line, header), line)
        }
    return DiffLines(lines, (all.size - limit).coerceAtLeast(0))
}

private fun kind(
    line: String,
    header: Boolean,
): DiffLineKind =
    when {
        line.startsWith("@@") -> DiffLineKind.Hunk
        header -> DiffLineKind.File
        line.startsWith("+") -> DiffLineKind.Added
        line.startsWith("-") -> DiffLineKind.Removed
        else -> DiffLineKind.Context
    }

/**
 * An edit's unified [diff] on a plate, in the terminal's face: added lines in verdigris, removed ones in oxide, the
 * headers and hunks quiet, each keeping its `+` or `-` so the color is never alone. It scrolls sideways rather than
 * wrapping; past [DIFF_LINE_LIMIT] lines it says how many more there are.
 */
@Composable
fun DiffView(
    diff: String,
    modifier: Modifier = Modifier,
) {
    val parsed = remember(diff) { diffLines(diff) }
    val colors =
        DiffColors(
            added = NgtermTheme.status.online,
            removed = NgtermTheme.status.error,
            quiet = MaterialTheme.colorScheme.onSurfaceVariant,
            text = MaterialTheme.colorScheme.onSurface,
        )
    val text =
        remember(parsed, colors) {
            buildAnnotatedString {
                parsed.lines.forEachIndexed { index, line ->
                    if (index > 0) append('\n')
                    withStyle(colors.style(line.kind)) { append(line.text) }
                }
            }
        }
    Column(modifier.fillMaxWidth()) {
        Text(
            text = text,
            style = ChatType.output,
            softWrap = false,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.small)
                    .horizontalScroll(rememberScrollState())
                    .padding(NgtermTheme.spacing.md),
        )
        if (parsed.more > 0) {
            Text(
                text = pluralStringResource(R.plurals.chat_diff_more, parsed.more, parsed.more),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = NgtermTheme.spacing.xs),
            )
        }
    }
}

// the colors a diff's lines take
private data class DiffColors(
    val added: Color,
    val removed: Color,
    val quiet: Color,
    val text: Color,
) {
    fun style(kind: DiffLineKind): SpanStyle =
        when (kind) {
            DiffLineKind.File -> SpanStyle(color = quiet, fontWeight = FontWeight.Bold)
            DiffLineKind.Hunk -> SpanStyle(color = quiet)
            DiffLineKind.Added -> SpanStyle(color = added)
            DiffLineKind.Removed -> SpanStyle(color = removed)
            DiffLineKind.Context -> SpanStyle(color = text)
        }
}
