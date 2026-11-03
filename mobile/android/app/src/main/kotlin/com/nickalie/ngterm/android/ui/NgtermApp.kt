package com.nickalie.ngterm.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

private const val MACHINES = "machines"
private const val SESSIONS = "sessions"
private const val SETTINGS = "settings"

/**
 * The app's top-level destinations: the machines you reach, the sessions you have attached across them, and
 * settings.
 */
val AppDestinations =
    listOf(
        NavDestination(MACHINES, R.string.nav_machines, R.drawable.ic_machines),
        NavDestination(SESSIONS, R.string.nav_sessions, R.drawable.ic_sessions),
        NavDestination(SETTINGS, R.string.nav_settings, R.drawable.ic_settings),
    )

/**
 * The app: [AdaptiveScaffold] over the top-level destinations. Until their screens land, each shows its empty state
 * (Settings: the loaded core's [coreVersion]). The selected destination survives a configuration change.
 */
@Composable
fun NgtermApp(
    coreVersion: String,
    modifier: Modifier = Modifier,
    layout: NavLayout = currentNavLayout(),
) {
    var selected by rememberSaveable { mutableStateOf(MACHINES) }
    AdaptiveScaffold(
        destinations = AppDestinations,
        selected = selected,
        onSelect = { selected = it },
        modifier = modifier,
        layout = layout,
    ) {
        val motion = NgtermTheme.motion
        AnimatedContent(
            targetState = selected,
            transitionSpec = { fadeIn(motion.standard()) togetherWith fadeOut(motion.quick()) },
            label = "destination",
        ) { destination ->
            Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
                Destination(destination, coreVersion, onSelect = { selected = it })
            }
        }
    }
}

@Composable
private fun Destination(
    key: String,
    coreVersion: String,
    onSelect: (String) -> Unit,
) {
    when (key) {
        MACHINES -> {
            EmptyState(
                icon = R.drawable.ic_machines,
                title = stringResource(R.string.machines_empty_title),
                body = stringResource(R.string.machines_empty_body),
                // Task 13's Add Machine screen takes this over
                action = PanelAction(stringResource(R.string.machines_add), R.drawable.ic_add) {},
            )
        }

        SESSIONS -> {
            EmptyState(
                icon = R.drawable.ic_sessions,
                title = stringResource(R.string.sessions_empty_title),
                body = stringResource(R.string.sessions_empty_body),
                action = PanelAction(stringResource(R.string.sessions_open_machines)) { onSelect(MACHINES) },
            )
        }

        else -> {
            CoreStatus(coreVersion)
        }
    }
}
