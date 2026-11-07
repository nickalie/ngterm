//! The chat commands on the actor ([`crate::chat`]): opening a shown pane's chat on its machine's connection, closing
//! it, the one-shot reads (an older page, the catalog) and a composer's upload, run as local tasks on the connection
//! as it was when posted.

use std::rc::Rc;

use ngterm_core::remote::tree::RemoteCommandResult;

use super::{Command, State, connection_of};
use crate::api::MobileError;
use crate::chat::{self, ChatKey, FETCH_DEADLINE, Job, LinkReport, Lookup, NOT_SHOWN, Reporter, upload};
use crate::pane::PaneRef;
use crate::sessions;
use crate::transport::Transport;

/// Whether `command` is a chat command, for [`command`].
pub(super) fn handles(command: &Command) -> bool {
    matches!(
        command,
        Command::OpenChat { .. }
            | Command::CloseChat(_)
            | Command::LoadOlder { .. }
            | Command::AgentCommands { .. }
            | Command::Upload { .. }
    )
}

/// Handles a chat command.
pub(super) fn command<T: Transport>(state: &Rc<State<T>>, command: Command) {
    match command {
        Command::OpenChat { pane, reply } => {
            let _ = reply.send(open(state, &pane));
        }
        Command::CloseChat(pane) => {
            if let Ok(key) = ChatKey::of(&pane) {
                state.chats.borrow_mut().close(&key);
            }
        }
        Command::LoadOlder { pane, reply } => {
            let job = ChatKey::of(&pane).and_then(|key| {
                let job = state.chats.borrow().older(&key)?;
                Ok((key, job))
            });
            let (key, job) = match job {
                Ok((key, Some(job))) => (key, job),
                Ok((_, None)) => return drop(reply.send(Ok(Vec::new()))),
                Err(error) => return drop(reply.send(Err(error))),
            };
            let state = Rc::clone(state);
            tokio::task::spawn_local(async move {
                let answer = run(&job).await.map(|output| chat::parse_events(&output));
                let answer = answer.and_then(|events| state.chats.borrow_mut().paged(&key, job.generation, events));
                let _ = reply.send(answer);
            });
        }
        Command::AgentCommands { pane, reply } => {
            let lookup = ChatKey::of(&pane).and_then(|key| {
                let connection = connection_of(state, &key.machine_id)?;
                let lookup = state.chats.borrow_mut().commands(&key, connection)?;
                Ok((key, lookup))
            });
            let (key, job) = match lookup {
                Ok((_, Lookup::Kept(commands))) => return drop(reply.send(Ok(commands))),
                Ok((key, Lookup::Fetch(job))) => (key, job),
                Err(error) => return drop(reply.send(Err(error))),
            };
            let state = Rc::clone(state);
            tokio::task::spawn_local(async move {
                let answer = run(&job)
                    .await
                    .and_then(|output| chat::parse_catalog(&output).map_err(|reason| MobileError::Remote { reason }));
                if let Ok(commands) = &answer {
                    state.chats.borrow_mut().fetched(&key, job.generation, commands);
                }
                let _ = reply.send(answer);
            });
        }
        Command::Upload {
            machine_id,
            name,
            bytes,
            progress,
            cancel,
            mut reply,
        } => {
            let connection = match connection_of(state, &machine_id) {
                Ok(connection) => connection,
                Err(error) => return drop(reply.send(Err(error))),
            };
            tokio::task::spawn_local(async move {
                let told = |sent, total| progress.sent(sent, total);
                let cancelled = async {
                    match &cancel {
                        Some(token) => tokio::select! {
                            () = reply.closed() => {}
                            () = token.cancelled() => {}
                        },
                        None => reply.closed().await,
                    }
                };
                let answer = upload::upload(&*connection, &name, &bytes, &told, cancelled).await;
                let _ = reply.send(answer);
            });
        }
        // The loop handles every other command itself.
        _ => {}
    }
}

/// Opens the chat of a shown pane on its machine's connection.
fn open<T: Transport>(state: &Rc<State<T>>, pane: &PaneRef) -> Result<(), MobileError> {
    let key = ChatKey::of(pane)?;
    let connection = connection_of(state, &key.machine_id)?;
    if !state
        .panes
        .borrow()
        .is_shown(&key.machine_id, &key.session_id, key.pane)
    {
        return Err(MobileError::Remote {
            reason: NOT_SHOWN.to_owned(),
        });
    }
    state.chats.borrow_mut().open(key, connection, reporter(state))
}

/// Where the chats' channels report: the actor's chats, once the channel's task has the report.
fn reporter<T: Transport>(state: &Rc<State<T>>) -> Reporter {
    let state = Rc::downgrade(state);
    Rc::new(move |launch, report: LinkReport| {
        let Some(state) = state.upgrade() else { return };
        let events = state.chats.borrow_mut().report(launch, report, sessions::now());
        state.emit_all(events);
    })
}

/// Runs a one-shot read: what it printed, or why it gave nothing.
async fn run<C: crate::transport::Connection>(job: &Job<C>) -> Result<String, MobileError> {
    let result = sessions::run(&*job.connection, &job.command, FETCH_DEADLINE)
        .await
        .map_err(|reason| MobileError::Remote { reason })?;
    if result.status != 0 {
        return Err(MobileError::Remote {
            reason: failure(&result),
        });
    }
    Ok(result.stdout)
}

/// Why a read failed, in the origin's words when it gave any.
fn failure(result: &RemoteCommandResult) -> String {
    let said = result.stderr.trim();
    let said = said.strip_prefix("Error: ").unwrap_or(said);
    if said.is_empty() {
        format!("exit {}", result.status)
    } else {
        said.to_owned()
    }
}
