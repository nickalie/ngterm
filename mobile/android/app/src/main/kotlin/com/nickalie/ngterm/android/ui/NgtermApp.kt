package com.nickalie.ngterm.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.machines.HostKeySheet
import com.nickalie.ngterm.android.ui.machines.MachinesRoute
import com.nickalie.ngterm.android.ui.machines.MachinesViewModel
import com.nickalie.ngterm.android.ui.machines.hostKeyQuestions
import com.nickalie.ngterm.android.ui.onboarding.OnboardingActions
import com.nickalie.ngterm.android.ui.onboarding.OnboardingFlow
import com.nickalie.ngterm.android.ui.onboarding.OnboardingStep
import com.nickalie.ngterm.android.ui.onboarding.OnboardingViewModel
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
 * The app over [graph]: the first run until it has a key, then [AdaptiveScaffold] over the top-level destinations,
 * with the host-key question as a sheet over whichever is shown. Sessions shows its empty state until its screen
 * lands; Settings the loaded core's [coreVersion]. The selected destination survives a configuration change.
 */
@Composable
fun NgtermApp(
    graph: AppGraph,
    coreVersion: String,
    modifier: Modifier = Modifier,
    layout: NavLayout = currentNavLayout(),
    onboarding: OnboardingViewModel = viewModel { OnboardingViewModel(graph.keys) },
    machines: MachinesViewModel = viewModel { MachinesViewModel(graph) },
) {
    val step by onboarding.step.collectAsStateWithLifecycle()
    when (step) {
        OnboardingStep.Loading -> {
            Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
        }

        OnboardingStep.Done -> {
            Frame(graph, coreVersion, layout, modifier)
            val state by machines.state.collectAsStateWithLifecycle()
            state.hostKeyQuestions().firstOrNull()?.let { (machine, question) ->
                key(machine.id, question.fingerprint) {
                    HostKeySheet(
                        machine = machine.title,
                        address = machine.address,
                        fingerprint = question.fingerprint,
                        onAnswer = { trust -> machines.answerHostKey(machine.id, question.fingerprint, trust) },
                    )
                }
            }
        }

        else -> {
            val actions = remember(onboarding) { OnboardingActions.of(onboarding) }
            OnboardingFlow(step, actions, modifier)
        }
    }
}

@Composable
private fun Frame(
    graph: AppGraph,
    coreVersion: String,
    layout: NavLayout,
    modifier: Modifier = Modifier,
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
            Destination(destination, graph, coreVersion, onSelect = { selected = it })
        }
    }
}

@Composable
private fun Destination(
    key: String,
    graph: AppGraph,
    coreVersion: String,
    onSelect: (String) -> Unit,
) {
    when (key) {
        MACHINES -> {
            MachinesRoute(graph)
        }

        SESSIONS -> {
            Centered {
                EmptyState(
                    icon = R.drawable.ic_sessions,
                    title = stringResource(R.string.sessions_empty_title),
                    body = stringResource(R.string.sessions_empty_body),
                    action = PanelAction(stringResource(R.string.sessions_open_machines)) { onSelect(MACHINES) },
                )
            }
        }

        else -> {
            Centered { CoreStatus(coreVersion) }
        }
    }
}

@Composable
private fun Centered(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) { content() }
}
