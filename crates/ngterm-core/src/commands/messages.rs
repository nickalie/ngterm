//! `msg.*` validation and app-global addressing (`ControlDispatcher+Messages.swift`, `ControlServer+Messages.swift`).

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use ngterm_proto::command::MsgCmd;
use ngterm_proto::{Args, ControlResult, MessageRejection, MessageRejectionReason, Request};

use super::{Outcome, args_of};
use crate::Core;
use crate::error::ControlError;
use crate::messages::{MAX_BATCH, MessageAddress, MessageRecipient, validate_body};
use crate::pane_env::uuid_string;
use crate::persist::parse_uuid_string;
use crate::resolve::Resolver;

pub(crate) fn dispatch(core: &mut Core, command: MsgCmd, request: &Request) -> Outcome {
    let args = args_of(request);
    match command {
        MsgCmd::Send => send(core, args),
        MsgCmd::Read => {
            let reader = require_identity(core, args, "msg.read")?;
            Ok(core
                .library
                .messages
                .read(reader.pane, MAX_BATCH, args.ack == Some(true)))
        }
        MsgCmd::Ack => {
            let reader = require_identity(core, args, "msg.ack")?;
            let raw = args
                .ids
                .as_ref()
                .filter(|ids| !ids.is_empty())
                .ok_or_else(|| ControlError::rejected("msg.ack requires message ids"))?;
            let ids = raw
                .iter()
                .map(|id| {
                    parse_uuid_string(id).ok_or_else(|| ControlError::rejected(format!("invalid message id: {id}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ControlResult {
                acked: Some(core.library.messages.ack(reader.pane, &ids)),
                ..ControlResult::default()
            })
        }
    }
}

fn resolve(core: &Core, text: &str, identity: bool) -> Result<MessageRecipient, ControlError> {
    let address = MessageAddress::parse(text, identity)?;
    let target = Resolver::new(core.library()).session(Some(address.session), None)?;
    core.library()
        .store(target.window)
        .and_then(|store| store.message_recipient(target.session, address.pane))
        .ok_or_else(|| {
            ControlError::rejected(format!(
                "pane {} is not a live pane of session {}",
                address.pane.text(),
                uuid_string(target.session)
            ))
        })
}

fn require_identity(core: &Core, args: &Args, command: &str) -> Result<MessageRecipient, ControlError> {
    let text = args
        .identity
        .as_deref()
        .ok_or_else(|| ControlError::rejected(format!("{command} requires an identity: --as SESSION:PANE-ID")))?;
    resolve(core, text, true)
}

fn send(core: &mut Core, args: &Args) -> Outcome {
    let body = args
        .body
        .as_deref()
        .ok_or_else(|| ControlError::rejected("msg.send requires a body"))?;
    let addresses = args.recipients.as_deref().unwrap_or_default();
    let all = args.all == Some(true);
    if all && !addresses.is_empty() {
        return Err(ControlError::rejected("msg.send takes --to or --all, not both"));
    }
    if !all && addresses.is_empty() {
        return Err(ControlError::rejected("msg.send requires --to or --all"));
    }
    if !all && args.workspace.is_some() {
        return Err(ControlError::rejected("msg.send: --workspace narrows --all only"));
    }
    validate_body(body)?;
    let sender = args
        .identity
        .as_deref()
        .map(|identity| resolve(core, identity, true))
        .transpose()?;
    let (mut targets, mut rejected) = recipients(core, args, all)?;
    let mut seen = HashSet::new();
    targets.retain(|target| {
        (!all || sender.as_ref().is_none_or(|sender| target.pane != sender.pane)) && seen.insert(target.pane)
    });
    let from = sender.as_ref().map(|sender| sender.address.as_str());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let mut result = core.library.messages.send(body, from, &targets, now)?;
    rejected.extend(result.rejected.take().unwrap_or_default());
    result.rejected = Some(rejected);
    let delivered = result.delivered.as_deref().unwrap_or_default();
    for target in targets.iter().filter(|target| delivered.contains(&target.address)) {
        for window in core.library.open_ids() {
            if let Some(store) = core.library.store_mut(window) {
                store.emit_message_queued(result.message_id.as_deref().unwrap_or_default(), from, target.pane);
            }
        }
    }
    Ok(result)
}

fn recipients(
    core: &Core,
    args: &Args,
    all: bool,
) -> Result<(Vec<MessageRecipient>, Vec<MessageRejection>), ControlError> {
    if all {
        let targets = if let Some(workspace) = &args.workspace {
            let target = Resolver::new(core.library()).workspace(Some(workspace), args.window.as_deref())?;
            core.library()
                .store(target.window)
                .map(|store| store.message_recipients(Some(target.workspace)))
                .unwrap_or_default()
        } else {
            core.library()
                .open_ids()
                .into_iter()
                .filter_map(|window| core.library().store(window))
                .flat_map(|store| store.message_recipients(None))
                .collect()
        };
        return Ok((targets, Vec::new()));
    }
    let mut targets = Vec::new();
    let mut rejected = Vec::new();
    for text in args.recipients.as_deref().unwrap_or_default() {
        MessageAddress::parse(text, false)?;
        match resolve(core, text, false) {
            Ok(recipient) => targets.push(recipient),
            Err(_) => rejected.push(MessageRejection {
                address: text.clone(),
                reason: MessageRejectionReason::UnknownPane,
            }),
        }
    }
    Ok((targets, rejected))
}
