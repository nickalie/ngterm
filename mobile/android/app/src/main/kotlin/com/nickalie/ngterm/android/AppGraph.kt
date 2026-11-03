package com.nickalie.ngterm.android

import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.ClipboardReads
import com.nickalie.ngterm.android.data.Connections
import com.nickalie.ngterm.android.data.MachineStore
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import kotlinx.coroutines.flow.StateFlow

/**
 * What the app's screens work with, made once per process: the machines, the keys, the pinned host keys, the
 * connections over the mobile core, the programs' clipboard reads, and whether the device is online.
 * [NgtermApplication] makes the real one; the tests make one over fakes.
 */
class AppGraph(
    val machines: MachineStore,
    val keys: AppKeys,
    val pins: PinnedHostKeys,
    val connections: Connections,
    val clipboardReads: ClipboardReads,
    val online: StateFlow<Boolean>,
)
