#if NGTERM_FAKES
    import NgtermCore

    /// One input call a ``FakePane`` took, with its arguments.
    public enum FakeInput: Equatable, Sendable {
        /// ``FakePane/clearSelection()``.
        case clearSelection
        /// ``FakePane/focus(focused:)``.
        case focus(Bool)
        /// ``FakePane/key(key:)``.
        case key(PaneKey)
        /// ``FakePane/mouse(mouse:)``.
        case mouse(PaneMouse)
        /// ``FakePane/paste(text:allowUnsafe:)``.
        case paste(text: String, allowUnsafe: Bool)
        /// ``FakePane/select(start:end:rectangle:)``.
        case select(start: CellPoint, end: CellPoint, rectangle: Bool)
        /// ``FakePane/selectAll()``.
        case selectAll
        /// ``FakePane/selectLine(at:)``.
        case selectLine(CellPoint)
        /// ``FakePane/selectWord(at:)``.
        case selectWord(CellPoint)
        /// ``FakePane/typeText(text:)``.
        case typed(String)
        /// ``FakePane/wheel(wheel:)``.
        case wheel(PaneWheel)
    }
#endif
