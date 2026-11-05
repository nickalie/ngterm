package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.core.ffi.RemoteLayout

// whether the window can show the origin's split: a wider window, two panes, the origin showing them both
internal fun splittable(
    layout: RemoteLayout?,
    compact: Boolean,
): Boolean = !compact && layout != null && layout.panes.size == 2 && layout.shown && layout.axis != null

/**
 * Whether the session shows its selected pane [alone] after a tap on the [tapped] pane's tab, where the window can show
 * the origin's split ([splittable]): a tap in the split shows that pane alone, a tap on the pane shown alone brings the
 * split back, and a tap on the other pane's tab shows that one alone instead. Where the split can't show, it stays.
 */
internal fun tabTapAlone(
    splittable: Boolean,
    alone: Boolean,
    tapped: String,
    focused: String?,
): Boolean =
    when {
        !splittable -> alone
        alone -> tapped != focused
        else -> true
    }

/**
 * A pane tab's tap: it selects the tapped pane ([focus]), and where the window can show the split ([splittable]) it
 * toggles the selected pane [alone] through [onAlone] as [tabTapAlone] says.
 */
internal fun tabTap(
    splittable: Boolean,
    alone: Boolean,
    focused: String?,
    onAlone: (Boolean) -> Unit,
    focus: (String) -> Unit,
): (String) -> Unit =
    { tapped ->
        val next = tabTapAlone(splittable, alone, tapped, focused)
        if (next != alone) onAlone(next)
        focus(tapped)
    }
