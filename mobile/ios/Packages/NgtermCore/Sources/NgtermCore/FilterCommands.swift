import NgtermCoreBindings

/// The `/` menu's rows for `query` (what follows the `/`): the commands whose name matches, best first, ties A to Z, as
/// the desktop palettes rank them (crates/ngterm-mobile, chat.rs's `filter_commands`); a blank query lists them all.
public func filterCommands(query: String, items: [AgentCommand]) -> [AgentCommand] {
    NgtermCoreBindings.filterCommands(query: query, items: items)
}
