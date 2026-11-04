import NgtermCoreBindings

// The core's own pane handle is the live ``PaneClient``: the protocol states its methods as the bindings have them.
extension Pane: PaneClient {}
