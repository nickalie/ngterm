//! One discovery followed by ordered companion attachments (upstream `ControlServer+RemoteGroup.swift`). A group
//! whose parent is a top-level page session starts with that page's handover, as a top-level row here.

use std::time::Duration;

use ngterm_proto::{
    ControlRemoteGroup, ControlRemoteGroupMember, ControlRemoteMember, ControlRemoteSession, ControlRemoteTree,
    ControlResult, Request, Response,
};
use uuid::Uuid;

use super::attach::{AttachRequest, Placement, start_tree_read};
use super::companions::{PageAttach, PageSpot, Step};
use super::origin_replies::read_remote_tree;
use super::tree;
use super::viewer::{Answer, take_answer};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::model::RemoteOrigin;
use crate::pane_env::uuid_string;
use crate::resolve::{Resolver, trimmed};

const POLL: Duration = Duration::from_millis(50);
const CLOSED: &str = "parent row was closed";

/// A group discovery or page handover that lets other control requests run meanwhile.
#[derive(Debug)]
pub(crate) struct GroupRun {
    request: AttachRequest,
    state: State,
}

#[derive(Debug)]
enum State {
    Discovering(Answer),
    /// The group's parent is a top-level page, whose handover runs before its members attach.
    Rooting(Box<Rooting>),
    Attaching(Box<Attached>),
}

#[derive(Debug)]
struct Rooting {
    tree: ControlRemoteTree,
    remote: ControlRemoteSession,
    page: PageAttach,
}

#[derive(Debug)]
struct Attached {
    tree: ControlRemoteTree,
    parent: Uuid,
    name: String,
    known: bool,
    members: Vec<ControlRemoteMember>,
    outcomes: Vec<ControlRemoteGroupMember>,
    page: Option<PageAttach>,
}

/// One nonblocking group step, also used by the Attach Remote picker.
pub(crate) enum GroupStep {
    Pending(GroupRun, Duration),
    Done(Response),
}

pub(crate) fn begin(core: &mut Core, request: &Request) -> Dispatched {
    let args = request.args.as_ref();
    let Some(host) = trimmed(args.and_then(|args| args.host.as_deref())) else {
        return Dispatched::done(Err(ControlError::rejected("live.attach.group requires a host")));
    };
    let Some(session) = trimmed(request.target.as_deref()) else {
        return Dispatched::done(Err(ControlError::rejected(
            "live.attach.group requires a remote session",
        )));
    };
    if !tree::is_plain(session) {
        return Dispatched::done(Err(ControlError::rejected(tree::INVALID_REMOTE_SESSION)));
    }
    let request = AttachRequest {
        host: host.to_owned(),
        session: session.to_owned(),
        window: trimmed(args.and_then(|args| args.window.as_deref())).map(str::to_owned),
    };
    match start(core, request) {
        Ok(run) => drive(core, run),
        Err(error) => Dispatched::done(Err(error)),
    }
}

pub(crate) fn start(core: &Core, request: AttachRequest) -> Result<GroupRun, ControlError> {
    let answer = start_tree_read(core, &request.host)?;
    Ok(GroupRun {
        request,
        state: State::Discovering(answer),
    })
}

fn drive(core: &mut Core, run: GroupRun) -> Dispatched {
    match step(core, run) {
        GroupStep::Done(reply) => Dispatched::Done(reply),
        GroupStep::Pending(run, after) => Dispatched::Pending {
            after,
            resume: Box::new(move |core| drive(core, run)),
        },
    }
}

pub(crate) fn step(core: &mut Core, mut run: GroupRun) -> GroupStep {
    match run.state {
        State::Discovering(ref answer) => {
            let Some(result) = take_answer(answer) else {
                return GroupStep::Pending(run, POLL);
            };
            run.state = match discovered(core, &run.request, &result) {
                Ok(state) => state,
                Err(error) => return GroupStep::Done(Response::failure(error.to_string())),
            };
            step(core, run)
        }
        State::Rooting(rooting) => {
            let Rooting { tree, remote, page } = *rooting;
            let result = match core.step_page_attach(page) {
                Step::Pending(page, after) => {
                    run.state = State::Rooting(Box::new(Rooting { tree, remote, page }));
                    return GroupStep::Pending(run, after);
                }
                Step::Done(result) => *result,
            };
            let parent = result.and_then(|result| {
                result
                    .id
                    .as_deref()
                    .and_then(super::frames::parse_uuid)
                    .ok_or_else(|| "attach failed".to_owned())
            });
            match parent {
                Ok(parent) => {
                    run.state = State::Attaching(Box::new(Attached::new(tree, &remote, parent)));
                    step(core, run)
                }
                Err(error) => GroupStep::Done(Response::failure(error)),
            }
        }
        State::Attaching(mut attached) => {
            if let Some(page) = attached.page.take() {
                match core.step_page_attach(page) {
                    Step::Pending(page, after) => {
                        attached.page = Some(page);
                        run.state = State::Attaching(attached);
                        return GroupStep::Pending(run, after);
                    }
                    Step::Done(result) => attached.record(*result),
                }
            }
            while attached.outcomes.len() < attached.members.len() {
                if let Some(page) = next_member(core, &run.request.host, &mut attached) {
                    attached.page = Some(page);
                    run.state = State::Attaching(attached);
                    return GroupStep::Pending(run, POLL);
                }
            }
            GroupStep::Done(attached.reply(core))
        }
    }
}

fn discovered(
    core: &mut Core,
    request: &AttachRequest,
    result: &super::tree::RemoteCommandResult,
) -> Result<State, ControlError> {
    let tree = read_remote_tree(result, &request.host).map_err(ControlError::rejected)?;
    let tree = tree.listing(&request.session).clone();
    let remote = tree
        .sessions
        .iter()
        .find(|row| row.id == request.session)
        .ok_or_else(|| {
            ControlError::rejected(format!("no attachable session {} on {}", request.session, request.host))
        })?
        .clone();
    if remote.parent.is_some() {
        return Err(ControlError::rejected(format!(
            "{} is a companion, not a group's parent",
            request.session
        )));
    }
    let window = Resolver::new(core.library()).open_placement_window(request.window.as_deref())?;
    let placement = core
        .workspace_placement(window, true)
        .ok_or_else(|| ControlError::rejected("no window to attach into"))?;
    if remote.content.as_deref() == Some("page") {
        let origin = RemoteOrigin {
            host: request.host.clone(),
            endpoint: tree.endpoint.clone(),
            session_name: remote.name.clone(),
        };
        let spot = PageSpot::Root {
            window,
            workspace: placement.workspace,
        };
        let page = core
            .start_page_attach(&remote.id, &request.host, spot, Some(origin), true)
            .map_err(ControlError::rejected)?;
        return Ok(State::Rooting(Box::new(Rooting { tree, remote, page })));
    }
    let result = core.attach_remote_session(&remote, &tree, &request.host, placement)?;
    let parent = result
        .id
        .as_deref()
        .and_then(super::frames::parse_uuid)
        .ok_or_else(|| ControlError::rejected("could not create the session"))?;
    Ok(State::Attaching(Box::new(Attached::new(tree, &remote, parent))))
}

fn next_member(core: &mut Core, host: &str, attached: &mut Attached) -> Option<PageAttach> {
    let member = &attached.members[attached.outcomes.len()];
    let Some(placement) = Placement::under(core.library(), attached.parent, false) else {
        attached.record(Err(CLOSED.to_owned()));
        return None;
    };
    let Some(row) = attached.tree.sessions.iter().find(|row| row.id == member.id) else {
        attached.record(Err(format!("unavailable on {host}")));
        return None;
    };
    let outcome = if row.content.as_deref() == Some("page") {
        let origin = RemoteOrigin {
            host: host.to_owned(),
            endpoint: attached.tree.endpoint.clone(),
            session_name: row.name.clone(),
        };
        match core.start_page_attach(&row.id, host, PageSpot::Companion(attached.parent), Some(origin), false) {
            Ok(page) => return Some(page),
            Err(reason) => Err(reason),
        }
    } else {
        core.attach_remote_session(row, &attached.tree, host, placement)
            .map_err(|error| error.to_string())
    };
    attached.record(outcome);
    None
}

impl Attached {
    /// The group of `remote`, one session of `tree`, whose row here is `parent`: its members, from the origin's own
    /// list when it gave one, else the listed sessions under it.
    fn new(tree: ControlRemoteTree, remote: &ControlRemoteSession, parent: Uuid) -> Self {
        let members = remote.companions.clone().unwrap_or_else(|| {
            tree.sessions
                .iter()
                .filter(|row| row.parent.as_deref() == Some(remote.id.as_str()))
                .map(|row| ControlRemoteMember {
                    id: row.id.clone(),
                    name: row.name.clone(),
                })
                .collect()
        });
        Self {
            parent,
            name: remote.name.clone(),
            known: remote.companions.is_some(),
            members,
            outcomes: Vec::new(),
            page: None,
            tree,
        }
    }

    fn record(&mut self, result: Result<ControlResult, String>) {
        let origin = self.members[self.outcomes.len()].id.clone();
        let (id, error) = match result {
            Ok(result) => match result.id {
                Some(id) => (Some(id), None),
                None => (None, Some("attach failed".to_owned())),
            },
            Err(error) => (None, Some(error)),
        };
        self.outcomes.push(ControlRemoteGroupMember { origin, id, error });
    }

    fn reply(self, core: &Core) -> Response {
        let missing: Vec<_> = self
            .members
            .iter()
            .zip(&self.outcomes)
            .filter(|(_, result)| result.id.is_none())
            .collect();
        let error = if !self.members.is_empty() && core.library().store_for_session(self.parent).is_none() {
            let names = missing
                .iter()
                .map(|(member, _)| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let tail = if names.is_empty() {
                String::new()
            } else {
                format!("; not attached: {names}")
            };
            Some(format!("parent row {} was closed{tail}", uuid_string(self.parent)))
        } else if missing.is_empty() {
            None
        } else {
            let reasons = missing
                .iter()
                .map(|(member, result)| {
                    format!(
                        "{} ({})",
                        member.name,
                        result.error.as_deref().unwrap_or("not attached")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "kept {} ({}) and {} companion(s); not attached: {reasons}",
                self.name,
                uuid_string(self.parent),
                self.outcomes.len() - missing.len()
            ))
        };
        let result = ControlResult {
            remote_group: Some(ControlRemoteGroup {
                membership_known: self.known,
                companions: self.outcomes,
            }),
            ..ControlResult::id(uuid_string(self.parent))
        };
        match serde_json::to_value(result) {
            Ok(result) => match error {
                Some(error) => Response::failure_with_result(error, result),
                None => Response::with_result(result),
            },
            Err(error) => Response::failure(error.to_string()),
        }
    }
}
