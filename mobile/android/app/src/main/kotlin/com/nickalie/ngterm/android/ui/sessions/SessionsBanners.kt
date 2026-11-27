package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.CatalogState

// what is said over the list or the empty state: the device is offline, the last refresh failed, the catalog is not
// live, why agterm there lists nothing (the fix is on the machine, so it has no action: a refresh lists again once it
// is made)
@Composable
internal fun Banners(
    state: MachineSessions,
    failure: String?,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm)) {
        if (state.offline) Banner(StatusTone.Offline, stringResource(R.string.machines_offline))
        if (failure != null) {
            Banner(
                tone = StatusTone.Error,
                message = stringResource(R.string.machine_sessions_failed_banner, failure),
                action = SectionAction(stringResource(R.string.action_retry), onRetry),
            )
        }
        state.catalog?.let { CatalogBanner(it, state.machine.title) }
        state.hint?.let { Banner(StatusTone.Attention, it) }
    }
}

// a catalog that is not live says why, in upstream's words; the core retries a lost one itself, so it has no action
@Composable
private fun CatalogBanner(
    catalog: CatalogState,
    machine: String,
) {
    when (catalog) {
        CatalogState.Connecting -> {
            Banner(StatusTone.Connecting, stringResource(R.string.machine_sessions_catalog_connecting, machine))
        }

        is CatalogState.Disconnected -> {
            Banner(StatusTone.Offline, stringResource(R.string.machine_sessions_catalog_disconnected, catalog.reason))
        }

        is CatalogState.Refused -> {
            Banner(StatusTone.Error, catalog.reason)
        }

        CatalogState.Connected -> {}
    }
}

internal val MachineSessions.banners: Boolean
    get() = offline || hint != null || (catalog != null && catalog != CatalogState.Connected)
