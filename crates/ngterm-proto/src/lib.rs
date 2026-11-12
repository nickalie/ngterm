//! Wire types of the ngterm control protocol: one newline-delimited JSON request per connection
//! (`{cmd, target?, args?}`) answered by one response (`{ok, result?, error?}`).

pub mod command;
pub mod events;
pub mod keymap;
pub mod recent;
pub mod request;
pub mod response;
pub mod result;
#[cfg(any(test, feature = "catalog-check"))]
pub mod rust_only;
pub mod tree;
pub mod version;

pub use command::{Command, Group, UnknownCommand};
pub use events::{ControlEvent, EventBatch, EventKind, EventPayload};
pub use keymap::{
    ControlHookEntry, ControlHooks, ControlKeymap, ControlKeymapAction, ControlKeymapCommand, ControlKeymapDiagnostic,
    ControlKeymapMenuItem,
};
pub use recent::{ControlRecentItem, ControlRecentOpen};
pub use request::{
    AgentApproval, AgentQuestion, AgentQuestionOption, AgentReport, Args, AskButton, AskChoice, MAX_REQUEST_LINE_BYTES,
    PickItem, Request,
};
pub use response::Response;
pub use result::{
    BusMessage, ControlAgent, ControlAgentApproval, ControlAskOutcome, ControlAskResult, ControlCursor,
    ControlHtmlPageOutcome, ControlHtmlPageOutcomeState, ControlLiveEndpoint, ControlLiveEntry, ControlLiveInventory,
    ControlLiveResetStatus, ControlPickOutcome, ControlPickResult, ControlRemoteGroup, ControlRemoteGroupMember,
    ControlRemoteMember, ControlRemotePane, ControlRemoteSession, ControlRemoteTree, ControlRestartReceipt,
    ControlRestoreStatus, ControlResult, ControlWindowFrame, ControlWindowNode, MessageRejection,
    MessageRejectionReason, RemotePeer,
};
pub use tree::{ControlSessionNode, ControlSurfaceNode, ControlTree, ControlWorkspaceNode};
pub use version::{AppIdentity, ConformanceProfile, PARITY_LEVEL, VersionResult};
