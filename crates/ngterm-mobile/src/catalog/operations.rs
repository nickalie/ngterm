//! The structural actions a phone sends a headless origin under an operation id (upstream's `RemoteCatalogActions`):
//! New Session, Rename Session and End Session. The origin runs each at most once and answers with a receipt.
//!
//! - A receipt settles the action: applied (at a catalog revision), no change, refused or fenced.
//! - A reply this app never got is asked about once with `catalog op OPID --epoch E --rev N`, which also fences an id
//!   the origin does not hold, so a command still on its way is refused when it lands. When that gets no answer, or
//!   the origin cannot tell, the outcome is unknown and the caller is told so: the catalog then shows what became of
//!   it.

use ngterm_core::remote::origin::OriginForm;
use ngterm_core::remote::origin_replies::{COMPANION_DEADLINE, end_session_arguments, reply_of};
use ngterm_core::remote::ssh::{control_tail, control_tail_with_operands};
use ngterm_proto::{CatalogReceipt, ControlResult, Response};

use crate::sessions::{new_session_arguments, run};
use crate::transport::Connection;

/// A structural action, with what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    NewSession { name: Option<String> },
    RenameSession { id: String, name: String },
    EndSession { id: String },
}

impl Operation {
    /// The action's name in the messages about it.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NewSession { .. } => "New Session",
            Self::RenameSession { .. } => "Rename Session",
            Self::EndSession { .. } => "End Session",
        }
    }

    /// The origin's `ngtermctl` words, before the operation id.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        match self {
            Self::NewSession { name } => new_session_arguments(name.as_deref()),
            Self::RenameSession { id, .. } => rename_arguments(id),
            Self::EndSession { id } => end_session_arguments(id).map(str::to_owned).to_vec(),
        }
    }

    /// What follows the words after `--`: user text that may start with `-`.
    #[must_use]
    pub fn operands(&self) -> Vec<String> {
        match self {
            Self::RenameSession { name, .. } => vec![name.clone()],
            Self::NewSession { .. } | Self::EndSession { .. } => Vec::new(),
        }
    }
}

/// `session rename --target ID`, whose name follows as an operand.
#[must_use]
pub fn rename_arguments(id: &str) -> Vec<String> {
    ["session", "rename", "--target", id].map(str::to_owned).to_vec()
}

/// The catalog position an action was decided at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub epoch: String,
    pub rev: i64,
}

/// What became of an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Settled {
    /// The origin ran it; its changes reach the catalog at `(epoch, rev)` when the receipt names a revision. `reply` is
    /// the command's own answer, `None` when only a lookup told.
    Applied {
        epoch: String,
        rev: Option<i64>,
        reply: Option<Box<Response<ControlResult>>>,
    },
    /// It changed nothing there.
    NoChange,
    /// It did not run, or what became of it is unknown: why, in words for the user.
    Failed(String),
}

/// The message for an action whose reply was lost and whose lookup got no answer (upstream's string).
#[must_use]
pub fn unknown_outcome(host: &str, label: &str) -> String {
    format!("{host} did not answer {label}; its outcome is unknown until {host} can be asked")
}

/// The message for an action the origin itself cannot tell the outcome of.
#[must_use]
pub fn untold_outcome(host: &str, label: &str) -> String {
    format!("{host} cannot tell what became of {label}")
}

/// The message for an action the origin fenced: it will never run.
#[must_use]
pub fn fenced(host: &str, label: &str) -> String {
    format!("{host} did not run {label}")
}

/// The line that runs `operation` under `op_id` at `position`.
#[must_use]
pub fn operation_line(form: OriginForm, operation: &Operation, op_id: &str, position: &Position) -> String {
    let mut arguments = operation.arguments();
    arguments.extend([
        "--op-id".to_owned(),
        op_id.to_owned(),
        "--epoch".to_owned(),
        position.epoch.clone(),
    ]);
    control_tail_with_operands(form, &arguments, &operation.operands())
}

/// The line that asks what became of `op_id`, sent at `position`.
#[must_use]
pub fn lookup_line(form: OriginForm, op_id: &str, position: &Position) -> String {
    let rev = position.rev.to_string();
    control_tail(
        form,
        &["catalog", "op", op_id, "--epoch", &position.epoch, "--rev", &rev],
    )
}

/// Runs `operation` on `host` under `op_id`, decided at `position`, and settles it.
pub async fn send<C: Connection>(
    connection: &C,
    (host, form): (&str, OriginForm),
    operation: &Operation,
    op_id: &str,
    position: &Position,
) -> Settled {
    let line = operation_line(form, operation, op_id, position);
    let result = match run(connection, &line, COMPANION_DEADLINE).await {
        Ok(result) => result,
        // never ran: the connection is gone
        Err(reason) => return Settled::Failed(reason),
    };
    let reply = reply_of(&result);
    let receipt = reply
        .as_ref()
        .and_then(|reply| reply.result.as_ref())
        .and_then(|result| result.receipt.clone())
        .filter(|receipt| receipt.op_id == op_id);
    match receipt {
        Some(receipt) => settle(host, operation.label(), &receipt, reply),
        None => look_up(connection, (host, form), operation.label(), op_id, position).await,
    }
}

/// Asks once what became of `op_id`.
async fn look_up<C: Connection>(
    connection: &C,
    (host, form): (&str, OriginForm),
    label: &str,
    op_id: &str,
    position: &Position,
) -> Settled {
    let answer = run(connection, &lookup_line(form, op_id, position), COMPANION_DEADLINE).await;
    let receipt = answer
        .ok()
        .as_ref()
        .and_then(reply_of)
        .and_then(|reply| reply.result)
        .and_then(|result| result.receipt)
        .filter(|receipt| receipt.op_id == op_id);
    match receipt {
        Some(receipt) => settle(host, label, &receipt, None),
        None => Settled::Failed(unknown_outcome(host, label)),
    }
}

/// What `receipt`, with the command's own `reply` when it came, says became of the action.
#[must_use]
pub fn settle(host: &str, label: &str, receipt: &CatalogReceipt, reply: Option<Response<ControlResult>>) -> Settled {
    match receipt.outcome.as_str() {
        "applied" => Settled::Applied {
            epoch: receipt.epoch.clone(),
            rev: receipt.rev,
            reply: reply.map(Box::new),
        },
        "noop" => Settled::NoChange,
        "refused" => Settled::Failed(
            reply
                .and_then(|reply| reply.error)
                .or_else(|| receipt.error.clone())
                .unwrap_or_else(|| format!("the remote command failed on {host}")),
        ),
        "fenced" => Settled::Failed(fenced(host, label)),
        _ => Settled::Failed(untold_outcome(host, label)),
    }
}
