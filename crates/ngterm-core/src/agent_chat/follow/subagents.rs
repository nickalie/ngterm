//! Finding a conversation's subagent transcripts.
//!
//! - Claude: `<session-id>/subagents/agent-<id>.jsonl` beside the main transcript, its parent the `Agent` tool use its
//!   `agent-<id>.meta.json` names. A transcript whose meta is not written yet is found on a later look.
//! - Codex: the rollout of each thread the conversation spawned (`rollout-<time>-<thread>.jsonl`), parented to the task
//!   tool that spawned it. It is looked for in the main rollout's day directory and the newest one in the `sessions`
//!   tree (a conversation running past midnight spawns there); a thread whose rollout is in neither is not followed.
//!   The parser knows only the spawns among the lines it read, so a [`Look::Scan`] also reads the first line of each
//!   rollout there no older than the main one, and takes every one whose `session_meta` names a `thread_spawn` from
//!   this conversation: one spawned before the lines read is parented to its thread id, the id the task's
//!   notification shows under when its spawn is not read (`CodexParser`'s orphan notification).

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::HEAD_CAP;
use crate::agent_chat::AgentKind;
use crate::agent_chat::claude::subagent_parent;

/// The most bytes of a `.meta.json` read.
const META_CAP: u64 = 64 * 1024;

/// The length of `rollout-YYYY-MM-DDTHH-MM-SS`, the part of a rollout's name that sorts by its start.
const ROLLOUT_TIME: usize = "rollout-2026-01-01T00-00-00".len();

/// A subagent's transcript.
#[derive(Debug)]
pub(crate) struct Found {
    /// What marks it found: its path for Claude, its thread for Codex.
    pub key: String,
    pub path: PathBuf,
    /// The task tool's id.
    pub parent: String,
    /// What names it among the conversation's files, for the ids of its lines that name none: its file stem for
    /// Claude, its thread for Codex.
    pub source: String,
}

/// How hard to look for Codex subagents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Look {
    /// Only the spawns the parser read.
    Spawned,
    /// Also every rollout spawned from this conversation, read or not.
    Scan,
}

/// The subagent transcripts of the conversation at `main` not in `known` (by [`Found::key`]). `spawned` is what the
/// Codex parser read: each thread spawned and its task tool's id.
pub(crate) fn find(
    agent: AgentKind,
    main: &Path,
    spawned: &[(String, String)],
    known: &HashSet<String>,
    look: Look,
) -> Vec<Found> {
    match agent {
        AgentKind::Claude => claude(main, known),
        AgentKind::Codex => codex(main, spawned, known, look),
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
        source: stem.to_owned(),
    })
}

fn codex(main: &Path, spawned: &[(String, String)], known: &HashSet<String>, look: Look) -> Vec<Found> {
    let pending: Vec<_> = spawned.iter().filter(|(thread, _)| !known.contains(thread)).collect();
    if pending.is_empty() && look == Look::Spawned {
        return Vec::new();
    }
    let rollouts: Vec<PathBuf> = codex_day_dirs(main)
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten().map(|entry| entry.path()))
        .filter(|path| rollout_name(path).is_some())
        .collect();
    let mut found: Vec<Found> = pending
        .into_iter()
        .filter_map(|(thread, task)| {
            let suffix = format!("-{thread}.jsonl");
            let path = rollouts
                .iter()
                .find(|path| rollout_name(path).is_some_and(|name| name.ends_with(&suffix)))?;
            Some(Found {
                key: thread.clone(),
                path: path.clone(),
                parent: task.clone(),
                source: thread.clone(),
            })
        })
        .collect();
    if look == Look::Scan {
        let mut scanned = scan(main, &rollouts, known, &found);
        scanned.sort_by(|a, b| a.path.cmp(&b.path));
        found.extend(scanned);
    }
    found
}

/// The rollouts among `rollouts` spawned from the conversation at `main`, not in `known` nor `found` yet, each
/// parented to its own thread.
fn scan(main: &Path, rollouts: &[PathBuf], known: &HashSet<String>, found: &[Found]) -> Vec<Found> {
    let Some(main_name) = rollout_name(main) else {
        return Vec::new();
    };
    let started = main_name.get(..ROLLOUT_TIME).unwrap_or(main_name);
    rollouts
        .iter()
        .filter(|path| {
            let name = rollout_name(path).unwrap_or_default();
            name != main_name && name.get(..ROLLOUT_TIME).unwrap_or(name) >= started
        })
        .filter_map(|path| {
            let thread = spawned_from(path, main_name)?;
            let taken = known.contains(&thread) || found.iter().any(|found| found.key == thread);
            (!taken).then(|| Found {
                key: thread.clone(),
                path: path.clone(),
                parent: thread.clone(),
                source: thread,
            })
        })
        .collect()
}

/// The thread of the rollout at `path` when its `session_meta` names a `thread_spawn` from the thread whose rollout is
/// `main_name`. A guardian review or another conversation's subagent is not one.
fn spawned_from(path: &Path, main_name: &str) -> Option<String> {
    let mut line = Vec::new();
    BufReader::new(File::open(path).ok()?.take(HEAD_CAP))
        .read_until(b'\n', &mut line)
        .ok()?;
    let meta: Value = serde_json::from_slice(&line).ok()?;
    let payload = meta.get("payload")?;
    let parent = payload
        .pointer("/source/subagent/thread_spawn/parent_thread_id")?
        .as_str()?;
    if !main_name.ends_with(&format!("-{parent}.jsonl")) {
        return None;
    }
    payload.get("id")?.as_str().map(str::to_owned)
}

/// The file name of a Codex rollout.
fn rollout_name(path: &Path) -> Option<&str> {
    let jsonl = path.extension().is_some_and(|extension| extension == "jsonl");
    path.file_name()
        .and_then(OsStr::to_str)
        .filter(|name| jsonl && name.starts_with("rollout-"))
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
