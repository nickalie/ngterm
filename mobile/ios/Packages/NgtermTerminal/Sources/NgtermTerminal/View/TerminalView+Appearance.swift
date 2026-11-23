import NgtermCore

extension TerminalView {
    /// Gives the pane the app's appearance, `dark` or light, which its terminal reports to programs that ask.
    ///
    /// The pane reports a change only, so the same appearance given again sends nothing.
    public func setAppearance(dark: Bool) {
        pane.setColorScheme(scheme: dark ? .dark : .light)
    }
}
