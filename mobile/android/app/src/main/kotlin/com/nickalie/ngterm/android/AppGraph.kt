package com.nickalie.ngterm.android

import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.Asks
import com.nickalie.ngterm.android.data.ClipboardReads
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.MachineStore
import com.nickalie.ngterm.android.data.NotificationPrefs
import com.nickalie.ngterm.android.data.OpenRequests
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.android.ui.session.SessionPanes
import kotlinx.coroutines.flow.StateFlow

/**
 * What the app's screens work with, made once per process: the machines, the keys, the pinned host keys, the
 * connections over the mobile core, the attached sessions' live state and their shown panes, the programs' clipboard
 * reads, the origins' open asks, the notification choices, where a notification's tap asked the app to go, and
 * whether the device is online.
 * [NgtermApplication] makes the real one; the tests make one over fakes.
 */
@Suppress("LongParameterList") // the process's one graph: a parameter per part the screens share
class AppGraph(
    val machines: MachineStore,
    val keys: AppKeys,
    val pins: PinnedHostKeys,
    val connections: Connections,
    val clipboardReads: ClipboardReads,
    val panes: SessionPanes,
    val asks: Asks,
    val notificationPrefs: NotificationPrefs,
    val opens: OpenRequests,
    val online: StateFlow<Boolean>,
)

/** The application, as what holds the process's [AppGraph]: the app's own, or a test's. */
interface GraphOwner {
    val graph: AppGraph
}
