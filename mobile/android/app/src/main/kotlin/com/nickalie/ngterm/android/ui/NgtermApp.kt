package com.nickalie.ngterm.android.ui

import androidx.activity.compose.LocalActivity
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.asks.AskPrompt
import com.nickalie.ngterm.android.ui.asks.NotificationPermissionPrompt
import com.nickalie.ngterm.android.ui.machines.HostKeySheet
import com.nickalie.ngterm.android.ui.machines.MachinesRoute
import com.nickalie.ngterm.android.ui.machines.MachinesViewModel
import com.nickalie.ngterm.android.ui.machines.hostKeyQuestions
import com.nickalie.ngterm.android.ui.onboarding.OnboardingActions
import com.nickalie.ngterm.android.ui.onboarding.OnboardingFlow
import com.nickalie.ngterm.android.ui.onboarding.OnboardingStep
import com.nickalie.ngterm.android.ui.onboarding.OnboardingViewModel
import com.nickalie.ngterm.android.ui.session.AttachedActions
import com.nickalie.ngterm.android.ui.session.AttachedUiState
import com.nickalie.ngterm.android.ui.session.KeepScreenOn
import com.nickalie.ngterm.android.ui.session.OpenSession
import com.nickalie.ngterm.android.ui.session.RailSessions
import com.nickalie.ngterm.android.ui.session.SessionViewModel
import com.nickalie.ngterm.android.ui.session.SessionsDestination
import com.nickalie.ngterm.android.ui.session.SidebarSessions
import com.nickalie.ngterm.android.ui.settings.SettingsRoute
import com.nickalie.ngterm.android.ui.terminal.ClipboardReadPrompt
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import kotlinx.coroutines.flow.catch

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

/** The Sessions destination's state, as [NgtermApp] collects it from its [SessionViewModel]. */
private class SessionsUi(
    val state: AttachedUiState,
    val panes: Map<PaneRef, TerminalState>,
    val failures: Map<PaneRef, String>,
    val notice: String?,
)

/**
 * The app over [graph]: the first run until it has a key, then [AdaptiveScaffold] over the top-level destinations,
 * with the host-key question, a program's clipboard read, the session on screen's ask and the notification permission
 * as sheets over whichever is shown. Sessions holds the attached sessions and the session view, and the rail and the
 * sidebar list the attached sessions on every destination; Settings sets them up and shows the loaded core's
 * [coreVersion], and an open session keeps the screen on when Settings asks for it. The
 * selected destination survives a configuration change. A notification's tap ([AppGraph.opens]) opens its session and
 * pane through the session view model, or the app's first destination.
 */
@Composable
fun NgtermApp(
    graph: AppGraph,
    coreVersion: String,
    modifier: Modifier = Modifier,
    layout: NavLayout = currentNavLayout(),
    onboarding: OnboardingViewModel = viewModel { OnboardingViewModel(graph.keys) },
    machines: MachinesViewModel = viewModel { MachinesViewModel(graph) },
    sessions: SessionViewModel = viewModel { SessionViewModel(graph) },
) {
    val step by onboarding.step.collectAsStateWithLifecycle()
    when (step) {
        OnboardingStep.Loading -> {
            Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
        }

        OnboardingStep.Done -> {
            var selected by rememberSaveable { mutableStateOf(MACHINES) }
            val attached by sessions.state.collectAsStateWithLifecycle()
            val panes by sessions.panes.collectAsStateWithLifecycle()
            val failures by sessions.paneFailures.collectAsStateWithLifecycle()
            val notice by sessions.notice.collectAsStateWithLifecycle()
            val actions = remember(sessions) { AttachedActions.of(sessions, onOpenMachines = { selected = MACHINES }) }
            val target by graph.opens.pending.collectAsStateWithLifecycle()
            LaunchedEffect(target) {
                target?.let { opened ->
                    selected = goTo(opened, sessions, graph.connections.attached.value)
                    graph.opens.consume(opened)
                }
            }
            OnScreen(graph, if (selected == SESSIONS) attached.open else null)
            Frame(
                graph = graph,
                coreVersion = coreVersion,
                layout = layout,
                selected = selected,
                onSelect = { selected = it },
                sessions = SessionsUi(attached, panes, failures, notice),
                actions = actions,
                modifier = modifier,
            )
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
            ClipboardReadPrompt(graph)
            AskPrompt(graph)
            NotificationPermissionPrompt(graph)
        }

        else -> {
            val actions = remember(onboarding) { OnboardingActions.of(onboarding) }
            OnboardingFlow(step, actions, modifier)
        }
    }
}

// opens a notification's target: its session (and pane) under Sessions while it is attached ([attached]); Machines for
// a machine's, or for a session no longer attached (detached since, or gone with a process the system ended)
private fun goTo(
    target: OpenTarget,
    sessions: SessionViewModel,
    attached: Map<String, Set<String>>,
): String {
    val session = target.sessionId?.takeIf { it in attached[target.machineId].orEmpty() } ?: return MACHINES
    val key = SessionKey(target.machineId, session)
    sessions.open(key)
    target.paneId?.let { sessions.focus(key, it) }
    return SESSIONS
}

// tells the asks which session is on screen: the open one while Sessions shows it and the app is in the foreground. An
// activity recreated for a configuration change (a rotation, a theme switch) keeps it: the new one restores it on its
// first frame, and letting it go in between would post the sheet's ask as a notification.
@Composable
private fun OnScreen(
    graph: AppGraph,
    open: OpenSession?,
) {
    val activity = LocalActivity.current
    val changing = { activity?.isChangingConfigurations == true }
    // collected whatever the state: collecting only while started would never see the app leave
    val state by LocalLifecycleOwner.current.lifecycle.currentStateFlow
        .collectAsState()
    val shown = (open as? OpenSession.Live)?.key?.takeIf { state.isAtLeast(Lifecycle.State.STARTED) }
    LaunchedEffect(shown) { if (shown != null || !changing()) graph.asks.showing(shown) }
    DisposableEffect(graph) { onDispose { if (!changing()) graph.asks.showing(null) } }
}

@Composable
private fun Frame(
    graph: AppGraph,
    coreVersion: String,
    layout: NavLayout,
    selected: String,
    onSelect: (String) -> Unit,
    sessions: SessionsUi,
    actions: AttachedActions,
    modifier: Modifier = Modifier,
) {
    val open = sessions.state.open
    val settingsFlow = remember(graph) { graph.settings.settings.catch { emit(AppSettings()) } }
    val settings by settingsFlow.collectAsStateWithLifecycle(AppSettings())
    if (selected == SESSIONS && open is OpenSession.Live && settings.keepScreenOn) KeepScreenOn()
    val openSession = { key: SessionKey ->
        actions.onOpen(key)
        onSelect(SESSIONS)
    }
    val groups = sessions.state.groups
    AdaptiveScaffold(
        destinations = AppDestinations,
        selected = selected,
        onSelect = onSelect,
        modifier = modifier,
        layout = layout,
        // a phone's open session has the whole screen
        navigationVisible = !(selected == SESSIONS && open != null),
        railContent =
            if (groups.isEmpty()) {
                null
            } else {
                { expanded -> RailSessions(groups, open?.key, expanded, openSession) }
            },
        sidebarContent = { SidebarSessions(groups, open?.key, openSession) },
    ) {
        val motion = NgtermTheme.motion
        AnimatedContent(
            targetState = selected,
            transitionSpec = { fadeIn(motion.standard()) togetherWith fadeOut(motion.quick()) },
            label = "destination",
        ) { destination ->
            when (destination) {
                MACHINES -> {
                    MachinesRoute(graph, onOpenSession = openSession)
                }

                SESSIONS -> {
                    SessionsDestination(
                        state = sessions.state,
                        panes = sessions.panes,
                        failures = sessions.failures,
                        compact = layout == NavLayout.Bar,
                        notice = sessions.notice,
                        actions = actions,
                        extraKeys = settings.extraKeys,
                    )
                }

                else -> {
                    SettingsRoute(graph, coreVersion)
                }
            }
        }
    }
}
