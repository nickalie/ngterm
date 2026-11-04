//! Finding a conversation's subagent transcripts.
//!
//! - Claude: `<session-id>/subagents/agent-<id>.jsonl` beside the main transcript, its parent the `Agent` tool use its
//!   `agent-<id>.meta.json` names. A transcript whose meta is not written yet is found on a later look.
//! - Codex: the rollout of each thread the conversation spawned (`rollout-<time>-<thread>.jsonl`), parented to the task
//!   tool that spawned it. It is looked for in the main rollout's day directory and the newest one in the `sessions`
//!   tree (a conversation running past midnight spawns there); a thread whose rollout is in neither is not followed.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::agent_chat::AgentKind;
use crate::agent_chat::claude::subagent_parent;

/// The most bytes of a `.meta.json` read.
const META_CAP: u64 = 64 * 1024;

/// A subagent's transcript.
#[derive(Debug)]
pub(crate) struct Found {
    /// What marks it found: its path for Claude, its thread for Codex.
    pub key: String,
    pub path: PathBuf,
    /// The task tool's id.
    pub parent: String,
}

/// The subagent transcripts of the conversation at `main` not in `known` (by [`Found::key`]). `spawned` is what the
/// Codex parser read: each thread spawned and its task tool's id.
pub(crate) fn find(agent: AgentKind, main: &Path, spawned: &[(String, String)], known: &HashSet<String>) -> Vec<Found> {
    match agent {
        AgentKind::Claude => claude(main, known),
        AgentKind::Codex => codex(main, spawned, known),
    }
}

fn claude(main: &Path, known: &HashSet<String>) -> Vec<Found> {
    let Ok(entries) = fs::read_dir(main.with_extension("").join("subagents")) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .flatten()
        .filter_map(|entry| claude_subagent(&entry.path(), known))
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn claude_subagent(path: &Path, known: &HashSet<String>) -> Option<Found> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    if !stem.starts_with("agent-") {
        return None;
    }
    let key = path.to_string_lossy().into_owned();
    if known.contains(&key) {
        return None;
    }
    let mut meta = Vec::new();
    File::open(path.with_file_name(format!("{stem}.meta.json")))
        .ok()?
        .take(META_CAP)
        .read_to_end(&mut meta)
        .ok()?;
    let parent = subagent_parent(&meta)?;
    Some(Found {
        key,
        path: path.to_owned(),
        parent,
    })
}

fn codex(main: &Path, spawned: &[(String, String)], known: &HashSet<String>) -> Vec<Found> {
    let pending: Vec<_> = spawned.iter().filter(|(thread, _)| !known.contains(thread)).collect();
    if pending.is_empty() {
        return Vec::new();
    }
    let rollouts: Vec<PathBuf> = codex_day_dirs(main)
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .collect();
    pending
        .into_iter()
        .filter_map(|(thread, task)| {
            let suffix = format!("-{thread}.jsonl");
            let path = rollouts.iter().find(|path| {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
            })?;
            Some(Found {
                key: thread.clone(),
                path: path.clone(),
                parent: task.clone(),
            })
        })
        .collect()
}

/// The main rollout's day directory, then the newest day directory of its `sessions/YYYY/MM/DD` tree.
fn codex_day_dirs(main: &Path) -> Vec<PathBuf> {
    let Some(day) = main.parent() else {
        return Vec::new();
    };
    let mut dirs = vec![day.to_owned()];
    let sessions = day.parent().and_then(Path::parent).and_then(Path::parent);
    let newest = sessions
        .and_then(newest_child)
        .and_then(|year| newest_child(&year))
        .and_then(|month| newest_child(&month));
    if let Some(newest) = newest.filter(|newest| newest != day) {
        dirs.push(newest);
    }
    dirs
}

/// The subdirectory of `dir` whose name sorts last.
fn newest_child(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .max()
}
