package com.nickalie.ngterm.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.FlowRowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.Cover
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.MachineCard
import com.nickalie.ngterm.android.ui.components.OfflineState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.components.SectionHeader
import com.nickalie.ngterm.android.ui.components.SessionRow
import com.nickalie.ngterm.android.ui.components.StatusBadge
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone

// The component sheets the screenshot tests render: every component of DESIGN.md's inventory in its main states,
// with real content. One column on a phone, two side by side from the medium width up.

@Composable
private fun Sheet(
    modifier: Modifier = Modifier,
    content: @Composable FlowRowScope.() -> Unit,
) {
    Surface(modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        BoxWithConstraints(Modifier.safeDrawingPadding().padding(NgtermTheme.spacing.lg)) {
            val columns = if (maxWidth >= 600.dp) 2 else 1
            FlowRow(
                maxItemsInEachRow = columns,
                horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xl),
                verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xl),
                content = content,
            )
        }
    }
}

@Composable
private fun Block(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        content = content,
    )
}

/** Status badges, section headers, machine cards and session rows. */
@Composable
fun RowsSheet(modifier: Modifier = Modifier) {
    Sheet(modifier) {
        Block(Modifier.weight(1f)) {
            SectionHeader("Machines", count = 3, action = SectionAction("Add") {})
            MachineCard("build-01", "nick@build-01.lan:22", StatusTone.Online, onClick = {}) {
                AgentStatusBadge("2 working", "#5fbfaa", "circle")
                AgentStatusBadge("1 waiting", "#f0904f", "diamond")
            }
            MachineCard(
                "studio",
                "nick@studio.tail0f3c.ts.net:22",
                StatusTone.Connecting,
                onClick = {},
                connectionLabel = "Reconnecting in 4 s",
            )
            MachineCard(
                "nas",
                "admin@192.168.1.20:2222",
                StatusTone.Error,
                onClick = {},
                connectionLabel = "Host key changed",
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.lg)) {
                StatusTone.entries.forEach { StatusBadge(it) }
            }
        }
        Block(Modifier.weight(1f)) {
            SectionHeader("Attached", count = 2)
            SessionRow(
                "api-refactor",
                onClick = {},
                context = "Split the auth module",
                attached = true,
                selected = true,
            ) {
                AgentStatusBadge("Working", "#e3b65a", "star")
            }
            SessionRow("api-refactor docs", onClick = {}, attached = true, companionOf = "api-refactor")
            SectionHeader("On this machine", action = SectionAction("New session") {})
            SessionRow("migrations", onClick = {}, context = "Waiting on review of 0042") {
                // an origin's color too dim for the ground: the badge lifts it
                AgentStatusBadge("Needs input", "#3a1010", "triangle")
            }
            SessionRow("scratch", onClick = {})
        }
    }
}

/** The banner, the empty, error and offline states, and the cover over a pane. */
@Composable
fun StatesSheet(modifier: Modifier = Modifier) {
    Sheet(modifier) {
        Block(Modifier.weight(1f)) {
            Banner(StatusTone.Connecting, "Reconnecting in 4 s", action = SectionAction("Retry now") {}, onDismiss = {})
            Banner(StatusTone.Error, "Showing the last list: build-01 stopped answering")
            EmptyState(
                icon = R.drawable.ic_machines,
                title = "No machines yet",
                body = "Add a server or desktop that runs ngterm, and drive its sessions from here over ssh.",
                action = PanelAction("Add machine", R.drawable.ic_add) {},
            )
            ErrorState(
                title = "ngtermctl isn't on build-01",
                body = "Install ngterm there and make sure ngtermctl is on the PATH of ssh logins.",
                action = PanelAction("Try again", R.drawable.ic_retry) {},
            )
        }
        Block(Modifier.weight(1f)) {
            OfflineState(
                title = "build-01 is out of reach",
                body = "Check the network or the machine. ngterm tries again in 30 s.",
                action = PanelAction("Try now") {},
            )
            CoveredPane()
        }
    }
}

@Composable
private fun CoveredPane(modifier: Modifier = Modifier) {
    val terminal = NgtermTheme.terminal
    Box(modifier.fillMaxWidth().height(320.dp).background(terminal.background, MaterialTheme.shapes.medium)) {
        Column(Modifier.padding(NgtermTheme.spacing.md)) {
            listOf("$ cargo test -p ngterm-mobile", "running 212 tests", "test rows::golden ... ok").forEach {
                Text(it, color = terminal.foreground, style = NgtermTheme.mono.medium)
            }
        }
        Cover(
            title = "Another device is driving this pane",
            body = "You see what it does. Take control to type here; the other device then watches.",
            action = SectionAction("Take control") {},
        )
    }
}
