//! The presentation stream's `agent` kind, this build's addition for the Android chat view
//! (`docs/plans/20261103-android-agent-chat.md`): what each pane of a presented session runs, as the origin's agent
//! record has it ([`crate::commands::agent`]).
//!
//! It goes only to a viewer whose hello lists [`AGENT_KIND`] (an opt-in kind, [`super::hub::OPT_IN_KINDS`]), so desktop
//! Attach Remote, which does not, gets the stream it always got, and a session no such viewer watches costs nothing.
//! The `tree` is untouched.
//!
//! Such a viewer gets one frame per pane (the main pane, and the split while it exists) right after the snapshot, then
//! one for a pane whenever what it runs changes: an agent starts, its conversation changes, or its record is cleared.
//! Building the frames is where the lazy foreground check runs ([`Core::presented_agent`]): on attach, on each commit
//! and on each heartbeat, since no event fires when an agent exits back to the shell.

use std::collections::BTreeMap;

use uuid::Uuid;

use super::frames::{Body, PresentationAgent, PresentationPane};
use super::hub::{AGENT_KIND, SubscriberId};
use crate::Core;

/// The most of a conversation id a frame carries, in bytes. The id comes from a hook's payload (a UUID in both agents),
/// and a frame over [`super::frames::MAX_FRAME_BYTES`] would close the viewer's stream, so a longer one is cut.
const MAX_CONVERSATION_BYTES: usize = 1024;

/// Each pane's frame, by pane identity.
pub(super) type AgentFrames = BTreeMap<Uuid, PresentationAgent>;

impl Core {
    /// The agent frame of each pane of `session` (the main pane, and the split while it exists), after the lazy
    /// foreground check; empty for a session in no open window or one attached from another machine.
    fn agent_frames(&mut self, session: Uuid) -> AgentFrames {
        let Some(found) = self
            .presented_session(session)
            .filter(|found| found.remote_host().is_none())
        else {
            return AgentFrames::new();
        };
        let mut panes = vec![found.pane_identity];
        if found.split.exists
            && let Some(split) = found.split_pane_identity
        {
            panes.push(split);
        }
        panes
            .into_iter()
            .map(|identity| {
                let running = self.presented_agent(identity);
                let frame = PresentationAgent {
                    pane: PresentationPane::Identity(identity),
                    agent: running.as_ref().map(|(agent, _)| agent.name().to_owned()),
                    conversation_id: running.map(|(_, mut conversation)| {
                        conversation.truncate(conversation.floor_char_boundary(MAX_CONVERSATION_BYTES));
                        conversation
                    }),
                };
                (identity, frame)
            })
            .collect()
    }

    /// Publishes to `session`'s agent viewers each pane whose frame differs from the one they last got, a pane new to
    /// them included, and remembers `frames` as what they got.
    fn publish_agent_changes(&mut self, session: Uuid, frames: AgentFrames) {
        let published = self.remote.published.entry(session).or_default();
        let changed: Vec<PresentationAgent> = frames
            .iter()
            .filter(|(identity, frame)| published.agents.get(identity) != Some(frame))
            .map(|(_, frame)| frame.clone())
            .collect();
        published.agents = frames;
        for frame in changed {
            self.remote.hub.publish(&Body::Agent(frame), session);
        }
    }

    /// Brings `session`'s agent viewers in line with its panes, when one watches it (commit and heartbeat).
    pub(super) fn publish_agents(&mut self, session: Uuid) {
        if !self.remote.hub.watched_for(AGENT_KIND, session) {
            return;
        }
        let frames = self.agent_frames(session);
        self.publish_agent_changes(session, frames);
    }

    /// The frames a viewer whose hello lists the agent kind starts from, read once for it and for the agent viewers
    /// already there: a record the lazy check clears now reaches them too, so what was published stays what they got.
    /// `None` for a viewer that does not list it.
    pub(super) fn attach_agents(&mut self, session: Uuid, kinds: &[String]) -> Option<AgentFrames> {
        if !kinds.iter().any(|kind| kind == AGENT_KIND) {
            return None;
        }
        let frames = self.agent_frames(session);
        self.publish_agent_changes(session, frames.clone());
        Some(frames)
    }

    /// Sends a new viewer its panes' frames, right after its snapshot.
    pub(super) fn send_attached_agents(&mut self, frames: AgentFrames, subscriber: SubscriberId) {
        for frame in frames.into_values() {
            if !self.remote.hub.send_to(Body::Agent(frame), subscriber) {
                return;
            }
        }
    }
}
