//! Operation receipts on a served catalog (upstream's `ServerActions+Receipts.swift`): a structural command sent with
//! an `opID` runs at most once, its outcome kept in the operation table and answered to every repeat, and `catalog.op`
//! reads that table, fencing an id it does not hold.

use ngterm_proto::command::{SessionCmd, WorkspaceCmd};
use ngterm_proto::{CatalogReceipt, Command, Request, Response};
use serde_json::Value;

use super::operations::{self, CatalogPosition, outcome};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::resolve::Resolver;

/// The served structural commands; any other command refuses an `opID`.
const RECEIPT_COMMANDS: [Command; 15] = [
    Command::Session(SessionCmd::New),
    Command::Session(SessionCmd::Split),
    Command::Session(SessionCmd::SplitClose),
    Command::Session(SessionCmd::Swap),
    Command::Session(SessionCmd::Rename),
    Command::Session(SessionCmd::Close),
    Command::Session(SessionCmd::Move),
    Command::Session(SessionCmd::CompanionNew),
    Command::Session(SessionCmd::CompanionPage),
    Command::Session(SessionCmd::Page),
    Command::Workspace(WorkspaceCmd::New),
    Command::Workspace(WorkspaceCmd::Rename),
    Command::Workspace(WorkspaceCmd::Delete),
    Command::Workspace(WorkspaceCmd::Move),
    Command::Session(SessionCmd::Scratch),
];

const PANES_CHANGED: &str = "the session's panes changed; take the catalog again";
const SCRATCH_CHANGED: &str = "the session's scratch changed; take the catalog again";

fn unsaved(op_id: &str) -> Response {
    Response::failure(format!(
        "the receipt for operation {op_id} could not be saved; ask catalog.op for it"
    ))
}

fn receipt_result(receipt: &CatalogReceipt) -> Value {
    serde_json::json!({ "receipt": receipt })
}

/// A receipt's answer: ok unless the operation was refused, fenced or unknown.
fn reply(receipt: &CatalogReceipt) -> Response {
    let error = match receipt.outcome.as_str() {
        outcome::REFUSED => receipt.error.clone().unwrap_or_default(),
        outcome::FENCED => format!("operation {} was fenced by a lookup and will not run", receipt.op_id),
        outcome::UNKNOWN => format!("operation {} has an unknown outcome and will not run", receipt.op_id),
        _ => return Response::with_result(receipt_result(receipt)),
    };
    Response::failure_with_result(error, receipt_result(receipt))
}

/// `response` with `receipt` in its result.
fn with_receipt(mut response: Response, receipt: &CatalogReceipt) -> Response {
    let value = serde_json::to_value(receipt).unwrap_or(Value::Null);
    match &mut response.result {
        Some(Value::Object(result)) => {
            result.insert("receipt".to_owned(), value);
        }
        _ => response.result = Some(receipt_result(receipt)),
    }
    response
}

/// Where an operation that runs now leaves the catalog's revisions.
struct Running {
    op_id: String,
    before: i64,
}

impl Core {
    fn catalog_position(&self) -> Option<CatalogPosition> {
        let served = self.catalog.served.as_ref()?;
        Some(CatalogPosition::new(served.hub.epoch(), served.hub.rev()))
    }

    /// Records `receipt` and writes it; whether it landed. A failed write keeps it in memory, so this run still answers
    /// the id the same way, but the caller is told nothing durable was kept.
    fn save_receipt(&mut self, receipt: CatalogReceipt) -> bool {
        if !self.record_receipt(receipt) {
            return false;
        }
        self.save_operations()
    }

    /// Records `receipt` in memory only; false without a served catalog.
    fn record_receipt(&mut self, receipt: CatalogReceipt) -> bool {
        let Some(position) = self.catalog_position() else {
            return false;
        };
        let Some(served) = self.catalog.served.as_mut() else {
            return false;
        };
        served.table.record(receipt, &position);
        true
    }

    /// Writes the open windows, then the table: upstream saves both in one store write, so an entry never reaches the
    /// disk before the model change it vouches for. Whether both landed.
    fn save_operations(&mut self) -> bool {
        let windows = self
            .persistence
            .as_mut()
            .is_none_or(|persistence| persistence.save_all_checked(&self.library));
        let Some(served) = self.catalog.served.as_mut() else {
            return false;
        };
        if !windows {
            served.unsaved = true;
            return false;
        }
        served.save()
    }

    /// Whether the disk holds what memory does, retrying a write that failed: an entry only memory holds is never
    /// handed out as a receipt, since a restart would lose it.
    fn operations_durable(&mut self) -> bool {
        if self.catalog.served.as_ref().is_some_and(|served| served.unsaved) {
            self.save_operations();
        }
        self.catalog.served.as_ref().is_some_and(|served| !served.unsaved)
    }

    /// The refusal of an operation decided on what `request`'s target no longer has: a page row a command cannot
    /// take, other panes, another scratch. A target that resolves to no session is left to the command's own error.
    fn operation_refusal(&self, request: &Request) -> Option<String> {
        if let Some(refusal) = crate::commands::companion_page::refusal(self, request) {
            return Some(refusal.to_string());
        }
        let args = request.args.as_ref()?;
        let session = Resolver::new(self.library())
            .session(request.target.as_deref(), args.window.as_deref())
            .ok()
            .and_then(|target| self.library().store(target.window)?.session(target.session))?;
        if let Some(expected) = &args.panes {
            let mut current = Vec::new();
            if session.content() != crate::model::SessionContent::Page {
                current.push(crate::pane_env::uuid_string(session.pane_identity));
            }
            if session.split.exists
                && let Some(split) = session.split_pane_identity
            {
                current.push(crate::pane_env::uuid_string(split));
            }
            let expected: Vec<String> = expected.iter().map(|pane| pane.to_uppercase()).collect();
            if current != expected {
                return Some(PANES_CHANGED.to_owned());
            }
        }
        // a session's scratch has no live identity of its own here yet: it reads `none`
        let scratch_changed = args
            .scratch
            .as_deref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case("none"));
        scratch_changed.then(|| SCRATCH_CHANGED.to_owned())
    }

    /// A command sent with `op_id` on a served catalog: answered from the table when the id is there, refused for
    /// another epoch or a changed target, and otherwise run once with its marker in the table while it runs.
    pub(crate) fn begin_operation(&mut self, request: &Request, op_id: &str) -> Dispatched {
        if !RECEIPT_COMMANDS.contains(&request.cmd) {
            return Dispatched::Done(Response::failure(format!("{} takes no operation id", request.cmd)));
        }
        if !CatalogReceipt::is_valid_op_id(op_id) {
            return Dispatched::Done(Response::failure("invalid operation id"));
        }
        let Some(served) = self.catalog.served.as_ref() else {
            return Dispatched::Done(Response::failure(crate::dispatch::OPERATION_RECEIPTS_ERROR));
        };
        if let Some(recorded) = served.table.receipt(op_id).cloned() {
            return Dispatched::Done(if self.operations_durable() {
                reply(&recorded)
            } else {
                unsaved(op_id)
            });
        }
        let epoch = served.hub.epoch().to_owned();
        let sent = request.args.as_ref().and_then(|args| args.epoch.as_deref());
        let refusal = if sent == Some(epoch.as_str()) {
            self.operation_refusal(request)
        } else {
            Some(format!(
                "the catalog is at epoch {epoch}; take its snapshot and send the command again"
            ))
        };
        if let Some(error) = refusal {
            let receipt = CatalogReceipt {
                error: Some(error),
                ..operations::receipt(op_id, &epoch, outcome::REFUSED)
            };
            let saved = self.save_receipt(receipt.clone());
            return Dispatched::Done(if saved { reply(&receipt) } else { unsaved(op_id) });
        }
        self.refresh_catalog();
        let before = self.catalog_position().map_or(0, |position| position.rev);
        // in memory only: a stop before the command lands must not leave an `applied` entry on disk
        self.record_receipt(operations::receipt(op_id, &epoch, outcome::APPLIED));
        let running = Running {
            op_id: op_id.to_owned(),
            before,
        };
        let dispatched = self.begin_plain(request);
        finish_operation(self, running, dispatched)
    }

    /// `catalog.op`: the table's receipt of `op_id`, or a fence recorded for it, `unknown` when a drop may have taken
    /// its entry since `sent`.
    pub(crate) fn catalog_operation(&mut self, op_id: &str, sent: Option<&CatalogPosition>) -> Response {
        let Some(served) = self.catalog.served.as_ref() else {
            return Response::failure(crate::dispatch::OPERATION_RECEIPTS_ERROR);
        };
        if let Some(recorded) = served.table.receipt(op_id).cloned() {
            if !self.operations_durable() {
                return unsaved(op_id);
            }
            return Response::with_result(receipt_result(&recorded));
        }
        let outcome = if served.table.may_have_dropped(sent) {
            outcome::UNKNOWN
        } else {
            outcome::FENCED
        };
        let fence = operations::receipt(op_id, served.hub.epoch(), outcome);
        if !self.save_receipt(fence.clone()) {
            return Response::failure(format!("could not save the fence for operation {op_id}"));
        }
        Response::with_result(receipt_result(&fence))
    }
}

/// Settles a running operation once its command answers: a step still pending carries the operation on.
fn finish_operation(core: &mut Core, running: Running, dispatched: Dispatched) -> Dispatched {
    match dispatched {
        Dispatched::Done(response) => Dispatched::Done(settle_operation(core, &running, response)),
        Dispatched::Pending { after, resume } => Dispatched::Pending {
            after,
            resume: Box::new(move |core: &mut Core| {
                let next = resume(core);
                finish_operation(core, running, next)
            }),
        },
    }
}

/// The receipt of a finished command: refused with its error, applied at the revision its changes reached, or noop.
fn settle_operation(core: &mut Core, running: &Running, response: Response) -> Response {
    core.refresh_catalog();
    let Some(position) = core.catalog_position() else {
        return response;
    };
    let receipt = if !response.ok {
        CatalogReceipt {
            error: response.error.clone(),
            ..operations::receipt(&running.op_id, &position.epoch, outcome::REFUSED)
        }
    } else if position.rev > running.before {
        CatalogReceipt {
            rev: Some(position.rev),
            ..operations::receipt(&running.op_id, &position.epoch, outcome::APPLIED)
        }
    } else {
        operations::receipt(&running.op_id, &position.epoch, outcome::NOOP)
    };
    if !core.save_receipt(receipt.clone()) {
        return unsaved(&running.op_id);
    }
    with_receipt(response, &receipt)
}
