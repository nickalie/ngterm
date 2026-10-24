//! `ask.open`, `ask.result` and `ask.cancel`: validation and its messages as upstream's `ControlDispatcher+Ask.swift`,
//! the effects as `agterm/Control/ControlServer+Ask.swift`.
//!
//! A terminal ask waits on its session, shown or not; a GUI ask holds its window's modal slot and, anchored to a
//! session, needs that session on screen. The dialog itself is the frontend's: it answers through
//! [`Core::answer_session_ask`] and [`Core::answer_window_ask`].

use std::collections::HashSet;

use ngterm_proto::command::AskCmd;
use ngterm_proto::{Args, AskButton, ControlAskOutcome, ControlAskResult, ControlResult, Request};
use uuid::Uuid;

use super::hud::resolve_pane_placement;
use super::{Outcome, args_of};
use crate::Core;
use crate::error::ControlError;
use crate::modal::AskOwner;
use crate::model::{
    AskAlignment, AskAnchor, AskChoiceForm, AskForm, AskInputForm, AskInputMode, AskStyle, MAX_ASK_BUTTONS,
    MAX_ASK_CHOICES, OverlayPane, PendingAsk,
};
use crate::pane_env::uuid_string;
use crate::remote::AskPlacement;
use crate::resolve::Resolver;
use crate::windows::WindowRequest;

/// The placement refusal of an ask over a scratch pane token.
const INVALID_PANE: &str = "ask pane must be left or right";

/// Answers an `ask.*` command.
pub(crate) fn dispatch(core: &mut Core, cmd: AskCmd, request: &Request) -> Outcome {
    let args = args_of(request);
    let window = args.window.as_deref();
    core.sync_modals();
    match cmd {
        AskCmd::Open => open(core, request.target.as_deref(), args),
        AskCmd::Result => {
            let id = request
                .target
                .as_deref()
                .ok_or_else(|| ControlError::rejected("ask.result requires an ask id"))?;
            let result = ask_result(core, id, window)?;
            Ok(ControlResult {
                ask: Some(result),
                ..ControlResult::default()
            })
        }
        AskCmd::Cancel => {
            let id = request
                .target
                .as_deref()
                .ok_or_else(|| ControlError::rejected("ask.cancel requires an ask id"))?;
            cancel(core, id, window)?;
            Ok(ControlResult::default())
        }
    }
}

// MARK: - validation

/// `args` checked as upstream's `dispatchAskOpen`, in its order, into the dialog to open and its pane selector.
fn validated(target: Option<&str>, args: &Args) -> Result<(PendingAsk, Option<OverlayPane>), ControlError> {
    let title = args
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .ok_or_else(|| ControlError::rejected("ask.open requires a title"))?;
    let form = parse_form(args)?;
    let roles = validated_buttons(args, form.as_ref(), title)?;
    let style = AskStyle::from_wire(args.style.as_deref().unwrap_or("terminal"))
        .ok_or_else(|| ControlError::rejected("unknown style"))?;
    let align = AskAlignment::from_wire(args.align.as_deref().unwrap_or("right"))
        .ok_or_else(|| ControlError::rejected("unknown align"))?;
    if args.width.is_some_and(|width| !(10..=100).contains(&width)) {
        return Err(ControlError::rejected("width must be 10 to 100"));
    }
    validate_hotkeys(&roles.buttons)?;
    if style == AskStyle::Gui && (args.pane.is_some() || args.pane_id.is_some()) && target.is_none() {
        return Err(ControlError::rejected("--pane requires a session"));
    }
    let pane = args
        .pane
        .as_deref()
        .map(|raw| {
            OverlayPane::from_control_name(raw).ok_or_else(|| ControlError::rejected("--pane must be left or right"))
        })
        .transpose()?;
    let ask = PendingAsk {
        id: uuid_string(Uuid::new_v4()),
        title: title.to_owned(),
        message: args.message.clone(),
        buttons: roles
            .buttons
            .into_iter()
            .map(|button| AskButton {
                hotkey: button.hotkey.map(|key| key.to_lowercase()),
                ..button
            })
            .collect(),
        default_id: roles.default_id,
        destructive_id: args.destructive_button.clone(),
        style,
        align,
        width: args.width,
        anchor: None,
        form,
        cancel_id: roles.cancel_id,
    };
    Ok((ask, pane))
}

/// The buttons a form gets when the caller names none (upstream's `askFormButtons`).
fn form_buttons() -> Vec<AskButton> {
    [("ok", "OK"), ("cancel", "Cancel")]
        .into_iter()
        .map(|(id, label)| AskButton {
            id: id.to_owned(),
            label: label.to_owned(),
            hotkey: None,
        })
        .collect()
}

/// The validated buttons and the roles that name them.
struct ButtonRoles {
    buttons: Vec<AskButton>,
    default_id: Option<String>,
    cancel_id: Option<String>,
}

/// The button list, generated for a form without caller buttons, checked with its roles: a generated pair defaults
/// to `ok` and cancels with `cancel`, and an input form takes no hotkeys.
fn validated_buttons(args: &Args, form: Option<&AskForm>, title: &str) -> Result<ButtonRoles, ControlError> {
    let generated = args.buttons.is_none() && form.is_some();
    let buttons = match &args.buttons {
        Some(buttons) => buttons.clone(),
        None if generated => form_buttons(),
        None => return Err(ControlError::rejected("ask.open requires buttons")),
    };
    let default_id = args
        .default_button
        .clone()
        .or_else(|| generated.then(|| "ok".to_owned()));
    let cancel_id = args
        .cancel_button
        .clone()
        .or_else(|| generated.then(|| "cancel".to_owned()));
    validate_button_list(&buttons, title, args.message.as_deref())?;
    let ids: HashSet<&str> = buttons.iter().map(|button| button.id.as_str()).collect();
    for (role, id) in [
        ("default", &default_id),
        ("destructive", &args.destructive_button),
        ("cancel", &cancel_id),
    ] {
        if let Some(id) = id
            && !ids.contains(id.as_str())
        {
            return Err(ControlError::rejected(format!("unknown {role} button: {id}")));
        }
    }
    if args.destructive_button.is_some() && default_id == args.destructive_button {
        return Err(ControlError::rejected("default button must not be destructive"));
    }
    if matches!(form, Some(AskForm::Input(_))) && buttons.iter().any(|button| button.hotkey.is_some()) {
        return Err(ControlError::rejected("ask input does not take hotkeys"));
    }
    Ok(ButtonRoles {
        buttons,
        default_id,
        cancel_id,
    })
}

/// The button list's own checks, and the control-character check over every text the dialog shows.
fn validate_button_list(buttons: &[AskButton], title: &str, message: Option<&str>) -> Result<(), ControlError> {
    if buttons.is_empty() {
        return Err(ControlError::rejected("ask.open requires at least one button"));
    }
    if buttons.len() > MAX_ASK_BUTTONS {
        return Err(ControlError::rejected(format!(
            "too many buttons (max {MAX_ASK_BUTTONS})"
        )));
    }
    if buttons.iter().any(|button| button.label.is_empty()) {
        return Err(ControlError::rejected("ask button label must not be empty"));
    }
    let mut ids = HashSet::new();
    if !buttons.iter().all(|button| ids.insert(button.id.as_str())) {
        return Err(ControlError::rejected("ask button ids must be unique"));
    }
    if contains_control_characters(title)
        || contains_control_characters(message.unwrap_or_default())
        || buttons.iter().any(|button| contains_control_characters(&button.label))
    {
        return Err(control_characters());
    }
    Ok(())
}

fn control_characters() -> ControlError {
    ControlError::rejected("ask text must not contain control characters")
}

/// The optional text field or choice list (upstream's `parseAskForm`), refusing the other form's arguments.
fn parse_form(args: &Args) -> Result<Option<AskForm>, ControlError> {
    if args.input.is_some() && args.choices.is_some() {
        return Err(ControlError::rejected("input and choices are exclusive"));
    }
    // each argument of one form, present while that form is absent
    let strays = [
        ("value", "input", args.input.is_none() && args.value.is_some()),
        (
            "placeholder",
            "input",
            args.input.is_none() && args.placeholder.is_some(),
        ),
        ("checked", "choices", args.choices.is_none() && args.checked.is_some()),
        ("multiple", "choices", args.choices.is_none() && args.multiple.is_some()),
        ("other", "choices", args.choices.is_none() && args.other.is_some()),
    ];
    if let Some((name, form, _)) = strays.iter().find(|(.., stray)| *stray) {
        return Err(ControlError::rejected(format!("{name} requires {form}")));
    }
    if let Some(input) = &args.input {
        return parse_input(
            input,
            args.value.as_deref().unwrap_or_default(),
            args.placeholder.as_deref(),
        )
        .map(Some);
    }
    if let Some(choices) = &args.choices {
        let list = AskChoiceForm {
            choices: choices.clone(),
            multiple: args.multiple.unwrap_or(false),
            checked: args.checked.clone().unwrap_or_default(),
            other: args.other.clone(),
        };
        validate_choices(&list)?;
        return Ok(Some(AskForm::Choices(list)));
    }
    Ok(None)
}

/// Upstream's `parseAskInput`: a known mode, the prefill within the cap, and each text's control rule.
fn parse_input(input: &str, value: &str, placeholder: Option<&str>) -> Result<AskForm, ControlError> {
    let mode = AskInputMode::from_wire(input).ok_or_else(|| ControlError::rejected("unknown input"))?;
    if value.len() > AskInputForm::MAX_TEXT_BYTES {
        return Err(ControlError::rejected(format!(
            "ask text too long (max {} bytes)",
            AskInputForm::MAX_TEXT_BYTES
        )));
    }
    if contains_control_characters(placeholder.unwrap_or_default()) || AskInputForm::has_forbidden_controls(value, mode)
    {
        return Err(control_characters());
    }
    Ok(AskForm::Input(AskInputForm {
        mode,
        value: value.to_owned(),
        placeholder: placeholder.map(str::to_owned),
    }))
}

/// Upstream's `parseAskChoices`, in its order.
fn validate_choices(list: &AskChoiceForm) -> Result<(), ControlError> {
    if !(1..=MAX_ASK_CHOICES).contains(&list.choices.len()) {
        return Err(ControlError::rejected(format!(
            "ask choices must be 1 to {MAX_ASK_CHOICES}"
        )));
    }
    if list.choices.iter().any(|choice| choice.label.is_empty()) {
        return Err(ControlError::rejected("ask choice label must not be empty"));
    }
    let mut ids = HashSet::new();
    if !list.choices.iter().all(|choice| ids.insert(choice.id.as_str())) {
        return Err(ControlError::rejected("ask choice ids must be unique"));
    }
    if list.choices.iter().any(|choice| {
        contains_control_characters(&choice.label)
            || choice
                .subtitle
                .as_deref()
                .is_some_and(|subtitle| subtitle.chars().any(|ch| ch != '\n' && is_control(ch)))
    }) {
        return Err(control_characters());
    }
    let mut seen = HashSet::new();
    if !list.checked.iter().all(|id| seen.insert(id.as_str())) {
        return Err(ControlError::rejected("ask checked ids must be unique"));
    }
    if let Some(unknown) = list.checked.iter().find(|id| !ids.contains(id.as_str())) {
        return Err(ControlError::rejected(format!("unknown checked choice: {unknown}")));
    }
    if !list.multiple && list.checked.len() > 1 {
        return Err(ControlError::rejected("radio choices accept one checked id"));
    }
    if let Some(other) = &list.other {
        if AskChoiceForm::is_blank(other) {
            return Err(ControlError::rejected("ask other placeholder must not be blank"));
        }
        if contains_control_characters(other) {
            return Err(control_characters());
        }
    }
    Ok(())
}

/// Each hotkey is one ASCII letter, unique case-insensitively.
fn validate_hotkeys(buttons: &[AskButton]) -> Result<(), ControlError> {
    let mut seen = HashSet::new();
    for hotkey in buttons.iter().filter_map(|button| button.hotkey.as_deref()) {
        if hotkey.len() != 1 || !hotkey.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(ControlError::rejected("ask button hotkey must be one ASCII letter"));
        }
        if !seen.insert(hotkey.to_ascii_lowercase()) {
            return Err(ControlError::rejected("ask button hotkeys must be unique"));
        }
    }
    Ok(())
}

/// Whether `text` carries a C0 control character or DEL (upstream's `containsControlCharacters`).
pub(crate) fn contains_control_characters(text: &str) -> bool {
    text.chars().any(is_control)
}

fn is_control(ch: char) -> bool {
    u32::from(ch) < 0x20 || ch == '\u{7f}'
}

// MARK: - open

/// Opens a validated ask: a terminal one on its session, a GUI one in its window's slot.
fn open(core: &mut Core, target: Option<&str>, args: &Args) -> Outcome {
    let (ask, pane) = validated(target, args)?;
    let placement = Placement {
        pane,
        pane_id: args.pane_id.as_deref(),
        follow: args.follow == Some(true),
    };
    let window = args.window.as_deref();
    let opened = if ask.style == AskStyle::Terminal {
        let resolved = Resolver::new(core.library()).session(target, window)?;
        open_terminal(core, ask, resolved.window, resolved.session, &placement)
    } else if target.is_some() {
        let resolved = Resolver::new(core.library()).session(target, window)?;
        open_gui(core, ask, resolved.window, Some(resolved.session), &placement)
    } else {
        let window = Resolver::new(core.library()).open_placement_window(window)?;
        open_gui(core, ask, window, None, &placement)
    };
    core.sync_modals();
    opened
}

/// Where the caller asked the dialog to sit, and whether to raise its window.
struct Placement<'a> {
    pane: Option<OverlayPane>,
    pane_id: Option<&'a str>,
    follow: bool,
}

/// Hands the ask to the viewer presenting `session`, or `None` to open it here.
fn present_remotely(
    core: &mut Core,
    ask: &PendingAsk,
    window: Uuid,
    session: Uuid,
    placement: &Placement<'_>,
) -> Option<Outcome> {
    let remote = AskPlacement {
        pane: placement.pane,
        pane_id: placement.pane_id,
        invalid_pane: INVALID_PANE,
    };
    crate::remote::present_ask_remotely(core, ask, window, session, &remote)
}

/// Upstream's `presentTerminalAsk`: the pane must be laid out, the session's slot free.
fn open_terminal(core: &mut Core, ask: PendingAsk, window: Uuid, session: Uuid, placement: &Placement<'_>) -> Outcome {
    if let Some(outcome) = present_remotely(core, &ask, window, session, placement) {
        return outcome;
    }
    let id = ask.id.clone();
    let store = core.library_mut().store_mut(window).ok_or_else(no_such_session)?;
    let target = store.session(session).ok_or_else(no_such_session)?;
    let resolved = resolve_pane_placement(target, placement.pane, placement.pane_id, true, INVALID_PANE)?;
    if !store.open_session_ask(session, ask, resolved.map(|(identity, _)| identity)) {
        return Err(ControlError::rejected("ask already pending"));
    }
    core.modals
        .register_ask(id.clone(), AskOwner::Session { session, window });
    follow(core, window, placement.follow);
    Ok(opened(id, resolved.map(|(_, pane)| pane)))
}

/// Upstream's `presentAsk`: a session anchor must be on screen (selected, no zoom, no dashboard), the window's slot
/// free.
fn open_gui(
    core: &mut Core,
    mut ask: PendingAsk,
    window: Uuid,
    session: Option<Uuid>,
    placement: &Placement<'_>,
) -> Outcome {
    if let Some(session) = session {
        if let Some(outcome) = present_remotely(core, &ask, window, session, placement) {
            return outcome;
        }
        let store = core.library().store(window).ok_or_else(no_such_session)?;
        let target = store.session(session).ok_or_else(no_such_session)?;
        if store.selected_session_id() != Some(session)
            || core.zoom_target(window).is_some()
            || core.dashboard(window).is_some()
        {
            return Err(ControlError::rejected("session not visible"));
        }
        let resolved = resolve_pane_placement(target, placement.pane, placement.pane_id, true, INVALID_PANE)?;
        ask.anchor = Some(AskAnchor {
            session,
            pane: resolved.map(|(_, pane)| pane),
            pane_identity: resolved.map(|(identity, _)| identity),
        });
    }
    let id = ask.id.clone();
    let pane = ask.anchor.as_ref().and_then(|anchor| anchor.pane);
    core.modals
        .open_window_ask(window, ask)
        .map_err(ControlError::rejected)?;
    follow(core, window, placement.follow);
    Ok(opened(id, pane))
}

/// `--follow`: raise the owning window and make it frontmost.
fn follow(core: &mut Core, window: Uuid, follow: bool) {
    if follow {
        core.request_window(WindowRequest::Raise(window));
        core.take_frontmost(window);
    }
}

fn opened(id: String, pane: Option<OverlayPane>) -> ControlResult {
    ControlResult {
        id: Some(id),
        pane: pane.map(|pane| pane.wire_name().to_owned()),
        ..ControlResult::default()
    }
}

fn no_such_session() -> ControlError {
    ControlError::rejected("no such session")
}

// MARK: - result and cancel

fn unknown_ask(id: &str) -> ControlError {
    ControlError::rejected(format!("unknown ask: {id}"))
}

/// The pending or retained outcome of ask `id`, scoped to the `--window` window when one is named (upstream's
/// `withAskResult`).
fn ask_result(core: &Core, id: &str, window: Option<&str>) -> Result<ControlAskResult, ControlError> {
    let (result, owner_window) = lookup(core, id).ok_or_else(|| unknown_ask(id))?;
    if window.is_some() && Resolver::new(core.library()).window_id(window)? != owner_window {
        return Err(unknown_ask(id));
    }
    Ok(result)
}

/// Ask `id`'s current outcome and its window: pending while its owner still holds it, else the retained outcome
/// (upstream's `AskRegistry.result`). A registered id its owner no longer holds is unknown.
fn lookup(core: &Core, id: &str) -> Option<(ControlAskResult, Uuid)> {
    let Some(owner) = core.modals.ask_owner(id) else {
        return core.modals.finished_ask(id);
    };
    let held = match owner {
        AskOwner::Window(window) => core.modals.window_ask(window).is_some_and(|ask| ask.id == id),
        AskOwner::Session { session, window } => core
            .library()
            .store(window)
            .and_then(|store| store.session(session))
            .and_then(|session| session.ask())
            .is_some_and(|pending| pending.ask.id == id),
    };
    held.then(|| (ControlAskResult::outcome(ControlAskOutcome::Pending), owner.window()))
}

/// Cancels ask `id` while it is pending; a finished one is a no-op that keeps its outcome.
fn cancel(core: &mut Core, id: &str, window: Option<&str>) -> Result<(), ControlError> {
    let result = ask_result(core, id, window)?;
    if result.result != ControlAskOutcome::Pending {
        return Ok(());
    }
    let cancelled = ControlAskResult::outcome(ControlAskOutcome::Cancelled);
    match core.modals.ask_owner(id) {
        Some(AskOwner::Window(window)) => {
            if core.modals.window_ask(window).is_some_and(|ask| ask.id == id) {
                core.modals.resolve_window_ask(window, cancelled);
            }
        }
        Some(AskOwner::Session { session, window }) => {
            if let Some(store) = core.library_mut().store_mut(window) {
                store.resolve_session_ask(session, id, cancelled);
            }
        }
        None => return Err(unknown_ask(id)),
    }
    core.sync_modals();
    Ok(())
}
