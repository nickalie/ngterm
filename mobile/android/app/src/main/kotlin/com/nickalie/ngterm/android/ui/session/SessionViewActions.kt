package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * What the session view does: [onBack] (a phone's, back to the list; `null` where the list stays beside it),
 * [onFocus] gives a pane the focus, [onShow] shows a pane at its slot's grid, [onTakeLead] is Take control, [onDetach]
 * and [onEnd] are the menu's (End confirmed first), and [onNextSession] and [onPreviousSession] are the keyboard's.
 */
data class SessionViewActions(
    val onBack: (() -> Unit)?,
    val onFocus: (SessionKey, String) -> Unit,
    val onShow: (PaneRef, PaneSize) -> Unit,
    val onTakeLead: (PaneRef) -> Unit,
    val onDetach: (SessionKey) -> Unit,
    val onEnd: (SessionKey) -> Unit,
    val onNextSession: () -> Unit,
    val onPreviousSession: () -> Unit,
)
