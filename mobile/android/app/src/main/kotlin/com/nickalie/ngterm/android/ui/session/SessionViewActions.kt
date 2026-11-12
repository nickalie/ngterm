package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.chat.PaneView
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * What the session view does: [onBack] (a phone's, back to the list; `null` where the list stays beside it),
 * [onFocus] gives a pane the focus, [onShow] shows a pane at its slot's grid ([onRetry] again, after it could not be),
 * [onTakeLead] is Take control, [onDetach], [onDuplicate], [onChatView] (Chat view) and [onEnd] are the menu's (End
 * confirmed first),
 * [onNextSession] and [onPreviousSession] are the keyboard's (`null` with no other session to go to: the chords are
 * then the program's), [onView] is the bar's Show chat and Show terminal, and
 * [onAlone] shows the session's selected pane alone (`true`) or the origin's split again (`false`), from a pane tab.
 */
data class SessionViewActions(
    val onBack: (() -> Unit)?,
    val onFocus: (SessionKey, String) -> Unit,
    val onShow: (PaneRef, PaneSize) -> Unit,
    val onRetry: (PaneRef, PaneSize) -> Unit,
    val onTakeLead: (PaneRef) -> Unit,
    val onDetach: (SessionKey) -> Unit,
    val onEnd: (SessionKey) -> Unit,
    val onNextSession: (() -> Unit)?,
    val onPreviousSession: (() -> Unit)?,
    val onView: (PaneRef, PaneView) -> Unit = { _, _ -> },
    val onAlone: (SessionKey, Boolean) -> Unit = { _, _ -> },
    val onChatView: (SessionKey, Boolean) -> Unit = { _, _ -> },
    val onDuplicate: (SessionKey) -> Unit = {},
)
