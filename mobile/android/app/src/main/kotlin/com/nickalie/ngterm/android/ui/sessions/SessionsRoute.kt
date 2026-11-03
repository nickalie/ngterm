package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nickalie.ngterm.android.AppGraph

/**
 * A machine's sessions, over its [SessionsViewModel]: the list, the New session dialog and the snackbar for what failed
 * out of sight (an End, an attach). It lists the machine afresh when it opens (not again on a rotation), and goes
 * [onBack] when the machine is removed meanwhile. [onReviewHostKey] opens the changed-key stop.
 */
@Composable
fun SessionsRoute(
    graph: AppGraph,
    machineId: String,
    onBack: () -> Unit,
    onReviewHostKey: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: SessionsViewModel = viewModel(key = "sessions:$machineId") { SessionsViewModel(graph, machineId) },
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val creating by viewModel.creating.collectAsStateWithLifecycle()
    val notice by viewModel.notice.collectAsStateWithLifecycle()
    var opened by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        if (!opened) {
            opened = true
            viewModel.opened()
        }
    }
    val back by rememberUpdatedState(onBack)
    val review by rememberUpdatedState(onReviewHostKey)
    LaunchedEffect(state is SessionsUiState.Gone) {
        if (state is SessionsUiState.Gone) back()
    }
    val uris = LocalUriHandler.current
    val actions =
        remember(viewModel) {
            SessionActions(
                onBack = { back() },
                onNewSession = viewModel::openNewSession,
                onRefresh = viewModel::refresh,
                onConnect = viewModel::connect,
                onReviewHostKey = { review() },
                onOpenReadme = { uris.openUri(SERVER_SETUP_URL) },
                onOpen = viewModel::attach,
                onDetach = viewModel::detach,
                onEnd = viewModel::end,
            )
        }
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(notice) {
        val text = notice ?: return@LaunchedEffect
        snackbar.showSnackbar(text, withDismissAction = true, duration = SnackbarDuration.Long)
        viewModel.noticeShown()
    }
    Box(modifier = modifier.fillMaxSize()) {
        val ready = state as? SessionsUiState.Ready
        if (ready != null) {
            SessionsScreen(ready.sessions, actions, now = graph.connections.clock)
            creating?.let {
                NewSessionDialog(
                    machine = ready.sessions.machine.title,
                    state = it,
                    onCreate = viewModel::create,
                    onDismiss = viewModel::closeNewSession,
                )
            }
        }
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).safeDrawingPadding())
    }
}
