//! The row context menus (upstream's `WorkspaceSidebar+ContextMenu.swift` `menu(forRow:)` and
//! `WorkspaceSidebar+TintMenu.swift`), as a pure model over a snapshot of the window the sidebar took on its last sync
//! (ported from the GTK build's `sidebar/menu.rs`), and the gpui-kit popup menu drawn from it ([`build`]).

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{
    Context, ElementId, IntoElement as _, ParentElement as _, SharedString, Styled as _, Text, Window, div, px,
};
use ngterm_core::model::{SidebarTint, TintChoice, TintColor, TintParts};
use uuid::Uuid;

/// What a session row's menu reads of its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    pub name: String,
    pub workspace: Uuid,
    pub flagged: bool,
    /// The agent status is not idle.
    pub has_status: bool,
    /// The focused pane's cwd, what Reveal in Files shows.
    pub cwd: String,
    /// A hidden overlay goes with the session, which its close confirmation names.
    pub hidden_overlays: bool,
    /// The row's tint, what the Tint submenu checks.
    pub tint: Option<SidebarTint>,
}

/// What a workspace row's menu reads of its workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFacts {
    pub id: Uuid,
    pub name: String,
    pub session_count: usize,
    pub focus_member: bool,
    /// The only marked workspace, with the filter applied (upstream's `isSoleFocus`).
    pub sole_focus: bool,
    /// Every session is flagged, the ones the flagged view hides included (upstream's `allSessionsFlagged`).
    pub all_flagged: bool,
    /// The row's own tint.
    pub tint: Option<SidebarTint>,
}

/// The window as the menus and the sidebar's other handlers see it, taken on each refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuContext {
    /// The multi-selection in visible order (empty unless it holds the selected session).
    pub selection: Vec<Uuid>,
    pub sessions: HashMap<Uuid, SessionFacts>,
    /// Every workspace in tree order.
    pub workspaces: Vec<WorkspaceFacts>,
    pub can_remove_workspace: bool,
}

impl MenuContext {
    /// What a session row's menu acts on: the selection when the clicked row is in it, else the clicked row (upstream's
    /// `sidebarSelectionTargets(forContextSession:)` for a rendered row).
    #[must_use]
    pub fn targets(&self, clicked: Uuid) -> Vec<Uuid> {
        if self.selection.contains(&clicked) {
            self.selection.clone()
        } else {
            vec![clicked]
        }
    }

    fn workspace(&self, id: Uuid) -> Option<&WorkspaceFacts> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    ClearStatus(Vec<Uuid>),
    MarkForAttention(Vec<Uuid>),
    /// Start the inline rename of a row.
    Rename(Uuid),
    /// Copy a name to the clipboard; `None` (a blank workspace name) leaves the clipboard alone.
    CopyName(Option<String>),
    Duplicate(Uuid),
    MoveTo {
        sessions: Vec<Uuid>,
        workspace: Uuid,
    },
    ToggleFlags(Vec<Uuid>),
    /// Reveal a directory in the file manager.
    Reveal(String),
    Close(Vec<Uuid>),
    NewSession(Uuid),
    OpenDirectory(Uuid),
    ToggleFocus(Uuid),
    FocusMembership {
        workspace: Uuid,
        member: bool,
    },
    /// Flag All Sessions / Unflag All Sessions.
    ToggleWorkspaceFlags(Uuid),
    DeleteWorkspace(Uuid),
    /// A Tint submenu choice for the session targets.
    Tint {
        sessions: Vec<Uuid>,
        choice: TintChoice,
    },
    /// A Tint submenu choice for a workspace row's own tint.
    TintWorkspace {
        workspace: Uuid,
        choice: TintChoice,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
    pub enabled: bool,
    /// Drawn with a check mark.
    pub checked: bool,
    /// A tint color's swatch drawn before the label.
    pub swatch: Option<TintColor>,
}

impl MenuItem {
    fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self {
            label: label.into(),
            action,
            enabled: true,
            checked: false,
            swatch: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item(MenuItem),
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
    /// A section title, not clickable.
    Header(String),
    Separator,
}

/// A session row's menu: Clear Status first while a target has a status and Mark for Attention while one is idle,
/// Rename and Copy Name and Duplicate Session for one target, Move to the workspaces the targets are not all in, Flag
/// or Unflag, Reveal for one target, Close.
#[must_use]
pub fn session_menu(context: &MenuContext, clicked: Uuid, reveal_enabled: impl Fn(&str) -> bool) -> Vec<MenuEntry> {
    let targets = context.targets(clicked);
    let facts: Vec<&SessionFacts> = targets.iter().filter_map(|id| context.sessions.get(id)).collect();
    let single = targets.len() == 1;
    let mut menu = Vec::new();
    if facts.iter().any(|facts| facts.has_status) {
        let label = if single { "Clear Status" } else { "Clear Statuses" };
        menu.push(MenuEntry::Item(MenuItem::new(
            label,
            MenuAction::ClearStatus(targets.clone()),
        )));
    }
    // the idle row's counterpart of Clear Status: `session status completed` on the idle targets
    if facts.iter().any(|facts| !facts.has_status) {
        menu.push(MenuEntry::Item(MenuItem::new(
            "Mark for Attention",
            MenuAction::MarkForAttention(targets.clone()),
        )));
    }
    if !facts.is_empty() {
        menu.push(MenuEntry::Separator);
    }
    if targets.len() <= 1 {
        menu.push(MenuEntry::Item(MenuItem::new("Rename", MenuAction::Rename(clicked))));
    }
    if single {
        let name = context.sessions.get(&clicked).map(|facts| facts.name.clone());
        menu.push(MenuEntry::Item(MenuItem::new("Copy Name", MenuAction::CopyName(name))));
        menu.push(MenuEntry::Item(MenuItem::new(
            "Duplicate Session",
            MenuAction::Duplicate(clicked),
        )));
    }
    let destinations: Vec<MenuEntry> = context
        .workspaces
        .iter()
        .filter(|workspace| facts.iter().any(|facts| facts.workspace != workspace.id))
        .map(|workspace| {
            MenuEntry::Item(MenuItem::new(
                workspace.name.clone(),
                MenuAction::MoveTo {
                    sessions: targets.clone(),
                    workspace: workspace.id,
                },
            ))
        })
        .collect();
    if !destinations.is_empty() {
        menu.push(MenuEntry::Submenu {
            label: "Move to".to_owned(),
            items: destinations,
        });
    }
    let all_flagged = !facts.is_empty() && facts.iter().all(|facts| facts.flagged);
    let flag = match (single, all_flagged) {
        (true, true) => "Unflag",
        (true, false) => "Flag",
        (false, true) => "Unflag Sessions",
        (false, false) => "Flag Sessions",
    };
    menu.push(MenuEntry::Item(MenuItem::new(
        flag,
        MenuAction::ToggleFlags(targets.clone()),
    )));
    menu.push(session_tint_submenu(context, &targets));
    if single && let Some(facts) = context.sessions.get(&clicked) {
        menu.push(MenuEntry::Item(MenuItem {
            enabled: reveal_enabled(&facts.cwd),
            ..MenuItem::new("Reveal in Files", MenuAction::Reveal(facts.cwd.clone()))
        }));
    }
    let close = if single {
        "Close Session".to_owned()
    } else {
        format!("Close {} Sessions", targets.len())
    };
    menu.push(MenuEntry::Item(MenuItem::new(close, MenuAction::Close(targets))));
    menu
}

/// A workspace row's menu: Rename, Copy Name, the session creators, the focus pair, Flag All Sessions or Unflag All
/// Sessions (disabled for an empty workspace), Delete Workspace (disabled at the last workspace).
#[must_use]
pub fn workspace_menu(context: &MenuContext, id: Uuid) -> Vec<MenuEntry> {
    let facts = context.workspace(id);
    let name = facts
        .map(|facts| facts.name.trim().to_owned())
        .filter(|name| !name.is_empty());
    let focused = facts.is_some_and(|facts| facts.sole_focus);
    let member = facts.is_some_and(|facts| facts.focus_member);
    let all_flagged = facts.is_some_and(|facts| facts.all_flagged);
    let has_sessions = facts.is_some_and(|facts| facts.session_count > 0);
    vec![
        MenuEntry::Item(MenuItem::new("Rename", MenuAction::Rename(id))),
        MenuEntry::Item(MenuItem::new("Copy Name", MenuAction::CopyName(name))),
        MenuEntry::Item(MenuItem::new("New Session", MenuAction::NewSession(id))),
        MenuEntry::Item(MenuItem::new("Open Directory…", MenuAction::OpenDirectory(id))),
        MenuEntry::Separator,
        MenuEntry::Item(MenuItem::new(
            if focused { "Unfocus" } else { "Focus" },
            MenuAction::ToggleFocus(id),
        )),
        MenuEntry::Item(MenuItem::new(
            if member { "Remove from Focus" } else { "Add to Focus" },
            MenuAction::FocusMembership {
                workspace: id,
                member: !member,
            },
        )),
        MenuEntry::Separator,
        // every session of the workspace, the ones the flagged tree layout hides included
        MenuEntry::Item(MenuItem {
            enabled: has_sessions,
            ..MenuItem::new(
                if all_flagged {
                    "Unflag All Sessions"
                } else {
                    "Flag All Sessions"
                },
                MenuAction::ToggleWorkspaceFlags(id),
            )
        }),
        tint_submenu(&[facts.and_then(|facts| facts.tint)], |choice| {
            MenuAction::TintWorkspace { workspace: id, choice }
        }),
        MenuEntry::Separator,
        MenuEntry::Item(MenuItem {
            enabled: context.can_remove_workspace,
            ..MenuItem::new("Delete Workspace", MenuAction::DeleteWorkspace(id))
        }),
    ]
}

/// The session targets' Tint submenu.
fn session_tint_submenu(context: &MenuContext, targets: &[Uuid]) -> MenuEntry {
    let tints: Vec<Option<SidebarTint>> = targets
        .iter()
        .map(|id| context.sessions.get(id).and_then(|facts| facts.tint))
        .collect();
    tint_submenu(&tints, |choice| MenuAction::Tint {
        sessions: targets.to_vec(),
        choice,
    })
}

/// The Tint submenu (upstream's `tintMenuItem`): None and the seven colors with their swatches, then Show On's
/// placements, enabled only while every target is tinted. An entry is checked only when every target shares it.
fn tint_submenu(current: &[Option<SidebarTint>], action: impl Fn(TintChoice) -> MenuAction) -> MenuEntry {
    let colors: Vec<Option<TintColor>> = current.iter().map(|tint| tint.map(SidebarTint::color)).collect();
    let shared = |color: Option<TintColor>| !colors.is_empty() && colors.iter().all(|each| *each == color);
    let mut items = vec![MenuEntry::Item(MenuItem {
        checked: shared(None),
        ..MenuItem::new("None", action(TintChoice::Color(None)))
    })];
    items.extend(TintColor::ALL.into_iter().map(|color| {
        MenuEntry::Item(MenuItem {
            checked: shared(Some(color)),
            swatch: Some(color),
            ..MenuItem::new(color.title(), action(TintChoice::Color(Some(color))))
        })
    }));
    items.push(MenuEntry::Separator);
    items.push(MenuEntry::Header("Show On".to_owned()));
    let all_tinted = current.iter().all(Option::is_some);
    let placed = |parts| !current.is_empty() && current.iter().all(|tint| tint.map(SidebarTint::parts) == Some(parts));
    for (title, parts) in [
        ("Icon", TintParts::ICON),
        ("Name", TintParts::NAME),
        ("Icon and Name", TintParts::BOTH),
    ] {
        items.push(MenuEntry::Item(MenuItem {
            enabled: all_tinted,
            checked: placed(parts),
            ..MenuItem::new(title, action(TintChoice::Parts(parts)))
        }));
    }
    MenuEntry::Submenu {
        label: "Tint".to_owned(),
        items,
    }
}

/// What a menu item runs its action with.
pub type Perform = Rc<dyn Fn(MenuAction, &mut Window, &mut gpui_kit::App)>;

/// Fills `menu` with `entries`, each item running `perform` with its action; a disabled item shows but does nothing.
pub fn build(
    mut menu: PopupMenu,
    entries: Vec<MenuEntry>,
    perform: &Perform,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for entry in entries {
        menu = match entry {
            MenuEntry::Separator => menu.separator(),
            MenuEntry::Header(label) => menu.label(label),
            MenuEntry::Item(item) => menu.item(item_of(item, perform)),
            MenuEntry::Submenu { label, items } => {
                let perform = Rc::clone(perform);
                menu.submenu(label, window, cx, move |submenu, window, cx| {
                    build(submenu, items.clone(), &perform, window, cx)
                })
            }
        };
    }
    menu
}

fn item_of(item: MenuItem, perform: &Perform) -> PopupMenuItem {
    let perform = Rc::clone(perform);
    let action = item.action;
    let entry = match item.swatch {
        // drawn as it shows, so the swatch follows the menu's light or dark side as the row it colors does; the label
        // is a Text with an id, so the item keeps its name for a screen reader (an element item carries none itself)
        Some(color) => {
            let label = SharedString::from(item.label);
            PopupMenuItem::element(move |_, cx| {
                let ink = super::look::tint(color, cx.theme().mode.is_dark());
                div()
                    .flex()
                    .items_center()
                    .gap(px(SWATCH_GAP))
                    .child(div().size(px(SWATCH)).rounded_full().bg(ink))
                    .child(Text::new(ElementId::from("tint-swatch-label"), label.clone()))
                    .into_any_element()
            })
        }
        None => PopupMenuItem::new(item.label),
    };
    entry
        .checked(item.checked)
        .disabled(!item.enabled)
        .on_click(move |_, window, cx| perform(action.clone(), window, cx))
}

/// A tint swatch's diameter and its gap before the label (upstream's 12-point image inset by 1).
const SWATCH: f32 = 10.0;
const SWATCH_GAP: f32 = 6.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) if item.enabled => item.label.clone(),
                MenuEntry::Item(item) => format!("({})", item.label),
                MenuEntry::Submenu { label, items } => format!("{label} [{}]", items.len()),
                MenuEntry::Header(label) => format!("# {label}"),
                MenuEntry::Separator => "-".to_owned(),
            })
            .collect()
    }

    /// Workspaces "one" (sessions a, b) and "two" (session c).
    fn context() -> (MenuContext, [Uuid; 2], [Uuid; 3]) {
        let [one, two] = [Uuid::new_v4(), Uuid::new_v4()];
        let [a, b, c] = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let facts = |name: &str, workspace| SessionFacts {
            name: name.to_owned(),
            workspace,
            flagged: false,
            has_status: false,
            cwd: "/tmp".to_owned(),
            hidden_overlays: false,
            tint: None,
        };
        let workspace = |id, name: &str, count| WorkspaceFacts {
            id,
            name: name.to_owned(),
            session_count: count,
            focus_member: false,
            sole_focus: false,
            all_flagged: false,
            tint: None,
        };
        let context = MenuContext {
            selection: Vec::new(),
            sessions: HashMap::from([(a, facts("a", one)), (b, facts("b", one)), (c, facts("c", two))]),
            workspaces: vec![workspace(one, "one", 2), workspace(two, "two", 1)],
            can_remove_workspace: true,
        };
        (context, [one, two], [a, b, c])
    }

    #[test]
    fn a_single_session_menu_offers_the_single_target_items() {
        let (context, [_, two], [a, ..]) = context();
        let menu = session_menu(&context, a, |_| true);
        assert_eq!(
            labels(&menu),
            [
                "Mark for Attention",
                "-",
                "Rename",
                "Copy Name",
                "Duplicate Session",
                "Move to [1]",
                "Flag",
                "Tint [13]",
                "Reveal in Files",
                "Close Session"
            ]
        );
        assert!(menu.contains(&MenuEntry::Submenu {
            label: "Move to".into(),
            items: vec![MenuEntry::Item(MenuItem::new(
                "two",
                MenuAction::MoveTo {
                    sessions: vec![a],
                    workspace: two
                }
            ))],
        }));
        assert_eq!(labels(&session_menu(&context, a, |_| false))[8], "(Reveal in Files)");
    }

    #[test]
    fn a_multi_selection_menu_acts_on_every_target() {
        let (mut context, _, [a, b, c]) = context();
        context.selection = vec![a, c];
        context.sessions.get_mut(&a).unwrap().flagged = true;
        context.sessions.get_mut(&c).unwrap().has_status = true;
        let menu = session_menu(&context, c, |_| true);
        assert_eq!(
            labels(&menu),
            [
                "Clear Statuses",
                "Mark for Attention",
                "-",
                "Move to [2]",
                "Flag Sessions",
                "Tint [13]",
                "Close 2 Sessions"
            ]
        );
        assert!(menu.contains(&MenuEntry::Item(MenuItem::new(
            "Close 2 Sessions",
            MenuAction::Close(vec![a, c])
        ))));
        // outside the selection the menu narrows to the clicked row
        assert_eq!(context.targets(b), [b]);
        context.sessions.get_mut(&c).unwrap().flagged = true;
        assert!(labels(&session_menu(&context, a, |_| true)).contains(&"Unflag Sessions".to_owned()));
    }

    /// `SidebarMarkForAttentionTests`: an idle row offers Mark for Attention and no Clear Status, a row with a status
    /// only Clear Status, a mixed batch both, each acting on every target (the store marks only the idle ones).
    #[test]
    fn mark_for_attention_sits_beside_clear_status_by_the_targets_statuses() {
        let (mut context, _, [a, b, c]) = context();
        let menu = session_menu(&context, a, |_| true);
        assert_eq!(labels(&menu)[..3], ["Mark for Attention", "-", "Rename"]);
        assert_eq!(
            menu[0],
            MenuEntry::Item(MenuItem::new(
                "Mark for Attention",
                MenuAction::MarkForAttention(vec![a])
            ))
        );

        context.sessions.get_mut(&a).unwrap().has_status = true;
        assert_eq!(
            labels(&session_menu(&context, a, |_| true))[..3],
            ["Clear Status", "-", "Rename"]
        );

        context.selection = vec![a, b];
        let menu = session_menu(&context, b, |_| true);
        assert_eq!(labels(&menu)[..3], ["Clear Statuses", "Mark for Attention", "-"]);
        assert!(menu.contains(&MenuEntry::Item(MenuItem::new(
            "Mark for Attention",
            MenuAction::MarkForAttention(vec![a, b])
        ))));
        assert!(labels(&menu).contains(&"Close 2 Sessions".to_owned()));

        // a row gone from the snapshot offers neither, and no separator
        context.selection.clear();
        context.sessions.remove(&c);
        assert_eq!(labels(&session_menu(&context, c, |_| true))[..1], ["Rename"]);
    }

    #[test]
    fn a_workspace_menu_reflects_focus_and_the_last_workspace() {
        let (mut context, [one, _], _) = context();
        assert_eq!(
            labels(&workspace_menu(&context, one)),
            [
                "Rename",
                "Copy Name",
                "New Session",
                "Open Directory…",
                "-",
                "Focus",
                "Add to Focus",
                "-",
                "Flag All Sessions",
                "Tint [13]",
                "-",
                "Delete Workspace"
            ]
        );
        context.workspaces[0].focus_member = true;
        context.workspaces[0].sole_focus = true;
        context.workspaces[0].name = "  ".into();
        context.can_remove_workspace = false;
        let menu = workspace_menu(&context, one);
        assert_eq!(
            labels(&menu)[5..],
            [
                "Unfocus",
                "Remove from Focus",
                "-",
                "Flag All Sessions",
                "Tint [13]",
                "-",
                "(Delete Workspace)"
            ]
        );
        assert_eq!(
            menu[1],
            MenuEntry::Item(MenuItem::new("Copy Name", MenuAction::CopyName(None)))
        );
    }

    /// `SidebarWorkspaceFlagTests`: Flag All Sessions until every session is flagged, then Unflag All Sessions, both
    /// acting on the clicked workspace; disabled for an empty workspace.
    #[test]
    fn a_workspace_menu_flags_or_unflags_all_its_sessions() {
        let (mut context, [one, two], _) = context();
        let flag_item = |context: &MenuContext, id| {
            workspace_menu(context, id)
                .into_iter()
                .find_map(|entry| match entry {
                    MenuEntry::Item(item) if item.action == MenuAction::ToggleWorkspaceFlags(id) => Some(item),
                    _ => None,
                })
                .unwrap()
        };
        let item = flag_item(&context, one);
        assert_eq!((item.label.as_str(), item.enabled), ("Flag All Sessions", true));
        context.workspaces[0].all_flagged = true;
        assert_eq!(flag_item(&context, one).label, "Unflag All Sessions");
        context.workspaces[1].session_count = 0;
        let item = flag_item(&context, two);
        assert_eq!((item.label.as_str(), item.enabled), ("Flag All Sessions", false));
    }

    /// The Tint submenu of a menu: its entries, `*` marking a checked one and `~` one with a swatch.
    fn tint_entries(menu: &[MenuEntry]) -> Vec<MenuEntry> {
        menu.iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu { label, items } if label == "Tint" => Some(items.clone()),
                _ => None,
            })
            .expect("a Tint submenu")
    }

    fn tint_labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => {
                    let check = if item.checked { "*" } else { "" };
                    let swatch = if item.swatch.is_some() { "~" } else { "" };
                    let label = if item.enabled {
                        item.label.clone()
                    } else {
                        format!("({})", item.label)
                    };
                    format!("{swatch}{label}{check}")
                }
                MenuEntry::Header(label) => format!("# {label}"),
                MenuEntry::Separator => "-".to_owned(),
                MenuEntry::Submenu { label, .. } => label.clone(),
            })
            .collect()
    }

    fn tint_action(entries: &[MenuEntry], label: &str) -> MenuAction {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == label => Some(item.action.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {label}"))
    }

    /// `testPickingAColorTintsTheClickedSession`, `testNoneClearsTheTint`: an untinted row checks None and disables
    /// Show On; a tinted one checks its color and placement; each entry acts on the clicked row.
    #[test]
    fn the_tint_submenu_checks_the_rows_tint_and_acts_on_it() {
        let (mut context, _, [a, ..]) = context();
        let entries = tint_entries(&session_menu(&context, a, |_| true));
        assert_eq!(
            tint_labels(&entries),
            [
                "None*",
                "~Red",
                "~Orange",
                "~Yellow",
                "~Green",
                "~Blue",
                "~Purple",
                "~Gray",
                "-",
                "# Show On",
                "(Icon)",
                "(Name)",
                "(Icon and Name)"
            ]
        );
        assert_eq!(
            tint_action(&entries, "Red"),
            MenuAction::Tint {
                sessions: vec![a],
                choice: TintChoice::Color(Some(TintColor::Red)),
            }
        );

        context.sessions.get_mut(&a).unwrap().tint = Some(SidebarTint::on_icon(TintColor::Red));
        let entries = tint_entries(&session_menu(&context, a, |_| true));
        let labels = tint_labels(&entries);
        assert_eq!((labels[0].as_str(), labels[1].as_str()), ("None", "~Red*"));
        assert_eq!(labels[10..], ["Icon*", "Name", "Icon and Name"]);
        assert_eq!(
            tint_action(&entries, "None"),
            MenuAction::Tint {
                sessions: vec![a],
                choice: TintChoice::Color(None),
            }
        );
    }

    /// `testABatchChecksOnlyWhatEveryTargetShares`, `testShowOnIsDisabledUnlessEveryTargetIsTinted`,
    /// `testShowOnMovesEachRowsOwnColor`.
    #[test]
    fn a_batch_checks_only_what_every_target_shares() {
        let (mut context, _, [a, b, _]) = context();
        context.selection = vec![a, b];
        context.sessions.get_mut(&a).unwrap().tint = Some(SidebarTint::new(TintColor::Red, TintParts::NAME));
        context.sessions.get_mut(&b).unwrap().tint = Some(SidebarTint::on_icon(TintColor::Green));
        let entries = tint_entries(&session_menu(&context, a, |_| true));
        assert!(
            tint_labels(&entries).iter().all(|label| !label.ends_with('*')),
            "mixed colors and placements check nothing"
        );
        assert_eq!(
            tint_action(&entries, "Icon and Name"),
            MenuAction::Tint {
                sessions: vec![a, b],
                choice: TintChoice::Parts(TintParts::BOTH),
            }
        );

        context.sessions.get_mut(&a).unwrap().tint = Some(SidebarTint::on_icon(TintColor::Blue));
        context.sessions.get_mut(&b).unwrap().tint = Some(SidebarTint::on_icon(TintColor::Blue));
        let labels = tint_labels(&tint_entries(&session_menu(&context, a, |_| true)));
        assert!(labels.contains(&"~Blue*".to_owned()) && labels.contains(&"Icon*".to_owned()));

        context.sessions.get_mut(&b).unwrap().tint = None;
        let labels = tint_labels(&tint_entries(&session_menu(&context, a, |_| true)));
        assert_eq!(
            labels[10..],
            ["(Icon)", "(Name)", "(Icon and Name)"],
            "one untinted target"
        );
    }

    /// `testWorkspaceMenuTintsTheWorkspaceRowAlone`.
    #[test]
    fn the_workspace_tint_submenu_acts_on_the_workspace_row() {
        let (mut context, [one, _], _) = context();
        context.workspaces[0].tint = Some(SidebarTint::new(TintColor::Purple, TintParts::BOTH));
        let entries = tint_entries(&workspace_menu(&context, one));
        let labels = tint_labels(&entries);
        assert_eq!(labels[6], "~Purple*");
        assert_eq!(labels[12], "Icon and Name*");
        assert_eq!(
            tint_action(&entries, "Gray"),
            MenuAction::TintWorkspace {
                workspace: one,
                choice: TintChoice::Color(Some(TintColor::Gray)),
            }
        );
    }
}
