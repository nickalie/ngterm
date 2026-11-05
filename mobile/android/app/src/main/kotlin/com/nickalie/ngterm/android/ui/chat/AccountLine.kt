package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatUsage
import java.time.Duration
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.util.Locale

/**
 * The account the agent is signed in with and how much of its weekly limit is used, over the chat's rows: the email
 * at the start, quiet; at the end "Weekly limit 72%" over a slim meter that stands still (verdigris, the attention
 * copper from 80%, oxide when it is spent) and when it resets, in [zone] as of [now]: the day and time within the
 * week, else the date. Either half shows alone when the origin read only that one. TalkBack reads it as one sentence.
 */
@Composable
fun AccountLine(
    account: ChatBody.Account,
    modifier: Modifier = Modifier,
    now: Instant = Instant.now(),
    zone: ZoneId = ZoneId.systemDefault(),
) {
    val spacing = NgtermTheme.spacing
    val weekly = account.weekly
    val resets = weekly?.resetsAt?.let { resetsAt(Instant.ofEpochSecond(it.toLong()), now, zone) }
    val spoken =
        listOfNotNull(
            account.email?.let { stringResource(R.string.chat_account_email_spoken, it) },
            weekly?.let { usage ->
                val used = stringResource(R.string.chat_account_limit_spoken, usage.usedPercent.toInt())
                resets?.let { stringResource(R.string.chat_account_resets_spoken, used, it) } ?: "$used."
            },
        ).joinToString(" ")
    Column(modifier.fillMaxWidth().clearAndSetSemantics { contentDescription = spoken }) {
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = 40.dp).padding(vertical = spacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.md),
        ) {
            Text(
                text = account.email.orEmpty(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            weekly?.let { WeeklyLimit(it, resets) }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

// the limit's words and meter, and when it resets under them
@Composable
private fun WeeklyLimit(
    usage: ChatUsage,
    resets: String?,
    modifier: Modifier = Modifier,
) {
    val used = usage.usedPercent.toInt().coerceIn(0, FULL)
    val tone = limitTone(used)
    Column(modifier, horizontalAlignment = Alignment.End) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        ) {
            Text(
                text = stringResource(R.string.chat_account_limit, used),
                style = MaterialTheme.typography.labelMedium,
                color = if (used >= HIGH) tone else MaterialTheme.colorScheme.onSurface,
            )
            Meter(used, tone)
        }
        resets?.let {
            Text(
                text = stringResource(R.string.chat_account_resets, it),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

// a slim track filled to [used] percent in [tone]
@Composable
private fun Meter(
    used: Int,
    tone: Color,
) {
    Box(
        Modifier
            .size(width = 56.dp, height = 4.dp)
            .clip(RoundedCornerShape(2.dp))
            .background(MaterialTheme.colorScheme.outlineVariant),
    ) {
        if (used > 0) {
            Box(
                Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(used / FULL.toFloat())
                    .background(tone),
            )
        }
    }
}

// verdigris while there is room, the attention copper from 80%, oxide once spent
@Composable
private fun limitTone(used: Int): Color {
    val status = NgtermTheme.status
    return when {
        used >= FULL -> status.error
        used >= HIGH -> status.attention
        else -> status.online
    }
}

private const val FULL = 100
private const val HIGH = 80

/**
 * When a limit resets ([at]), as of [now] in [zone]: the short day and the locale's short time within six days
 * ("Mon 14:00"), else the locale's medium date ("Nov 14, 2026").
 */
internal fun resetsAt(
    at: Instant,
    now: Instant,
    zone: ZoneId,
    locale: Locale = Locale.getDefault(),
): String {
    val time = at.atZone(zone)
    return if (Duration.between(now, at) <= Duration.ofDays(WEEK_DAYS)) {
        val day = time.dayOfWeek.getDisplayName(TextStyle.SHORT, locale)
        "$day ${DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale).format(time)}"
    } else {
        DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale).format(time)
    }
}

// within this many days a reset reads as a weekday
private const val WEEK_DAYS = 6L
