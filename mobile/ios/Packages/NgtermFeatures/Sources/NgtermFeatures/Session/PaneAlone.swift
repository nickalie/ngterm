import NgtermCore

/// A pane tab's other job (mobile/android's `PaneAlone.kt`): where the window can show the origin's split, a tab
/// toggles it, on this device alone.
internal enum PaneAlone {
    /// What a session's pane tabs do to its split.
    internal enum Split {
        /// The window shows the selected pane alone, and a tab on it brings the split back.
        case alone
        /// The window shows the split, and a tab shows its pane alone.
        case both
        /// The window can't show the split: a tab only selects.
        case fixed
    }

    /// Whether the window can show the origin's split: a regular width, two panes, the origin showing them both.
    internal static func isSplittable(_ layout: RemoteLayout?, isCompact: Bool) -> Bool {
        guard !isCompact, let layout else {
            return false
        }
        return layout.panes.count == SplitMetrics.panes && layout.shown && layout.axis != nil
    }

    /// Whether the session shows its selected pane alone after a tap on the `tapped` pane's tab.
    ///
    /// Where the window can show the split, a tap in the split shows that pane alone, a tap on the pane shown alone
    /// brings the split back, and a tap on the other pane's tab shows that one alone instead. Where the split can't
    /// show, it stays.
    internal static func afterTap(isSplittable: Bool, isAlone: Bool, tapped: String, focused: String?) -> Bool {
        guard isSplittable else {
            return isAlone
        }
        return isAlone ? tapped != focused : true
    }
}
