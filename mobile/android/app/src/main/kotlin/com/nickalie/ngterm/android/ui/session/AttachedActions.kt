package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.chat.PaneView
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * What the Sessions destination does, over its [SessionViewModel]: open and close a session's view, give a pane the
 * focus, show a pane (and try one again), Take control, Detach, Duplicate session ([onDuplicate]), End session, go to
 * the Machines destination ([onOpenMachines]), say the snackbar was shown ([onNoticeShown]), and show a pane as its
 * chat or its terminal ([onView]), and set a session's own chat view ([onChatView]).
 */
data class AttachedActions(
    val onOpen: (SessionKey) -> Unit,
    val onClose: () -> Unit,
    val onFocus: (SessionKey, String) -> Unit,
    val onShow: (PaneRef, PaneSize) -> Unit,
    val onRetry: (PaneRef, PaneSize) -> Unit,
    val onTakeLead: (PaneRef) -> Unit,
    val onDetach: (SessionKey) -> Unit,
    val onEnd: (SessionKey) -> Unit,
    val onOpenMachines: () -> Unit,
    val onNoticeShown: () -> Unit,
    val onView: (PaneRef, PaneView) -> Unit = { _, _ -> },
    val onChatView: (SessionKey, Boolean) -> Unit = { _, _ -> },
    val onDuplicate: (SessionKey) -> Unit = {},
) {
    companion object {
        /** The actions of [viewModel], with [onOpenMachines] going to Machines. */
        fun of(
            viewModel: SessionViewModel,
            onOpenMachines: () -> Unit,
        ) = AttachedActions(
            onOpen = viewModel::open,
            onClose = viewModel::close,
            onFocus = viewModel::focus,
            onShow = viewModel::show,
            onRetry = viewModel::retry,
            onTakeLead = viewModel::takeLead,
            onDetach = viewModel::detach,
            onEnd = viewModel::end,
            onOpenMachines = onOpenMachines,
            onNoticeShown = viewModel::noticeShown,
            onView = viewModel::pickView,
            onChatView = viewModel::setChatView,
            onDuplicate = viewModel::duplicate,
        )
    }
}
