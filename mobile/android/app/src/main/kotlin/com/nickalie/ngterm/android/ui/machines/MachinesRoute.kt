package com.nickalie.ngterm.android.ui.machines

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.sessions.SessionsRoute

/** A screen of the Machines destination, above the list. */
private sealed interface MachinesPage {
    val route: String

    data object Add : MachinesPage {
        override val route = "add"
    }

    data class Edit(
        val id: String,
    ) : MachinesPage {
        override val route = "edit:$id"
    }

    data class Sessions(
        val id: String,
    ) : MachinesPage {
        override val route = "sessions:$id"
    }

    data class HostKeyChanged(
        val id: String,
    ) : MachinesPage {
        override val route = "changed:$id"
    }

    data class PublicKey(
        val keyId: String,
    ) : MachinesPage {
        override val route = "key:$keyId"
    }

    companion object {
        fun of(route: String): MachinesPage? {
            val arg = route.substringAfter(':')
            return when (route.substringBefore(':')) {
                "add" -> Add
                "edit" -> Edit(arg)
                "sessions" -> Sessions(arg)
                "changed" -> HostKeyChanged(arg)
                "key" -> PublicKey(arg)
                else -> null
            }
        }
    }
}

/**
 * The Machines destination: the home and the screens it opens (a machine's sessions, Add and Edit machine, a key's
 * public half, a changed host key), stacked, with back returning down the stack, and the removal's Undo. The stack
 * and each screen's fields survive a configuration change and a trip up the stack and back.
 */
@Composable
fun MachinesRoute(
    graph: AppGraph,
    modifier: Modifier = Modifier,
    viewModel: MachinesViewModel = viewModel { MachinesViewModel(graph) },
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val keys by viewModel.keys.collectAsStateWithLifecycle()
    val pins by viewModel.pins.collectAsStateWithLifecycle()
    val removed by viewModel.removed.collectAsStateWithLifecycle()
    // the routes above the home, bottom first, joined by '|' (ids are UUIDs)
    var stack by rememberSaveable { mutableStateOf("") }
    val routes = stack.split('|').filter(String::isNotEmpty)
    val holder = rememberSaveableStateHolder()
    val push = { page: MachinesPage -> stack = (routes + page.route).joinToString("|") }
    val pop = {
        routes.lastOrNull()?.let(holder::removeState)
        stack = routes.dropLast(1).joinToString("|")
    }
    BackHandler(enabled = routes.isNotEmpty(), onBack = pop)
    val actions =
        MachineActions(
            onAdd = { push(MachinesPage.Add) },
            onOpen = { push(MachinesPage.Sessions(it)) },
            onEdit = { push(MachinesPage.Edit(it)) },
            onConnect = viewModel::connect,
            onDisconnect = viewModel::disconnect,
            onRemove = viewModel::remove,
            onReviewHostKey = { push(MachinesPage.HostKeyChanged(it)) },
            onRetryRead = viewModel::retry,
        )
    val snackbar = remember { SnackbarHostState() }
    val removedText = removed?.let { stringResource(R.string.machines_removed, it.machine.title) }
    val undo = stringResource(R.string.action_undo)
    LaunchedEffect(removed) {
        if (removedText == null) return@LaunchedEffect
        val result = snackbar.showSnackbar(removedText, actionLabel = undo, duration = SnackbarDuration.Long)
        if (result == SnackbarResult.ActionPerformed) viewModel.undoRemove() else viewModel.forgetRemoval()
    }
    Box(modifier = modifier.fillMaxSize()) {
        val top = routes.lastOrNull()
        holder.SaveableStateProvider(top ?: "home") {
            Page(
                graph = graph,
                page = top?.let(MachinesPage::of),
                state = state,
                actions = actions,
                keys = keys,
                pins = pins,
                onSave = { draft, editing ->
                    viewModel.save(draft, editing)
                    pop()
                },
                onRemovePin = { id ->
                    viewModel.removePin(id)
                    pop()
                },
                onShowKey = { push(MachinesPage.PublicKey(it)) },
                onPop = pop,
            )
        }
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).safeDrawingPadding())
    }
}

@Composable
private fun Page(
    graph: AppGraph,
    page: MachinesPage?,
    state: MachinesUiState,
    actions: MachineActions,
    keys: List<AppKey>,
    pins: Map<String, String>,
    onSave: (MachineDraft, String?) -> Unit,
    onRemovePin: (String) -> Unit,
    onShowKey: (String) -> Unit,
    onPop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val machine = (page as? MachinesPage.Edit)?.id ?: (page as? MachinesPage.HostKeyChanged)?.id
    val item = (state as? MachinesUiState.Ready)?.items?.firstOrNull { it.machine.id == machine }
    when (page) {
        null -> {
            MachinesScreen(state, actions, graph.connections.clock, modifier)
        }

        is MachinesPage.Sessions -> {
            SessionsRoute(
                graph = graph,
                machineId = page.id,
                onBack = onPop,
                onReviewHostKey = { actions.onReviewHostKey(page.id) },
                modifier = modifier,
            )
        }

        MachinesPage.Add -> {
            MachineFormScreen(
                initial = MachineDraft(),
                editing = false,
                keys = keys,
                pinned = null,
                onSave = { onSave(it, null) },
                onClose = onPop,
                onShowKey = onShowKey,
                modifier = modifier,
            )
        }

        is MachinesPage.Edit -> {
            if (item == null) {
                Gone(state, onPop)
            } else {
                MachineFormScreen(
                    initial = MachineDraft.of(item.machine),
                    editing = true,
                    keys = keys,
                    pinned = pins[item.machine.id],
                    onSave = { onSave(it, item.machine.id) },
                    onClose = onPop,
                    onShowKey = onShowKey,
                    modifier = modifier,
                )
            }
        }

        is MachinesPage.HostKeyChanged -> {
            ChangedKeyPage(item, state, onRemovePin, onPop, modifier)
        }

        is MachinesPage.PublicKey -> {
            KeyPage(keys.firstOrNull { it.id == page.keyId }, state, onPop, modifier)
        }
    }
}

// a changed host key's hard stop, while the machine's key is still the changed one
@Composable
private fun ChangedKeyPage(
    item: MachineItem?,
    state: MachinesUiState,
    onRemovePin: (String) -> Unit,
    onPop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val changed = item?.link as? Link.HostKeyChanged
    if (item == null || changed == null) {
        Gone(state, onPop)
    } else {
        HostKeyChangedScreen(
            machine = item.machine.title,
            pinned = changed.pinned,
            offered = changed.offered,
            onBack = onPop,
            onRemovePin = { onRemovePin(item.machine.id) },
            modifier = modifier,
        )
    }
}

// a key's public half, from the form
@Composable
private fun KeyPage(
    key: AppKey?,
    state: MachinesUiState,
    onPop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (key == null) {
        Gone(state, onPop)
    } else {
        PublicKeyScreen(
            key = key,
            modifier = modifier,
            exit = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back), onPop),
        )
    }
}

// what a page is about has gone (removed, or its question answered elsewhere): back to what is under it, once the
// list has been read (before that, the page may only not be known yet)
@Composable
private fun Gone(
    state: MachinesUiState,
    onPop: () -> Unit,
) {
    val pop by rememberUpdatedState(onPop)
    LaunchedEffect(state is MachinesUiState.Loading) {
        if (state !is MachinesUiState.Loading) pop()
    }
}
