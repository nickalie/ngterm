// swiftlint:disable:this file_name - the file holds the core's type aliases, none of which names it
import NgtermCoreBindings

// The core's records, enums and errors, under the facade's names: the app reaches the core through NgtermCore only,
// and the generated bindings stay this package's own. Their docs are the Rust ones (crates/ngterm-mobile, api.rs and
// the modules it names), which UniFFI copies into the bindings.

/// One entry of the chat composer's `/` menu: an agent's slash command or skill.
public typealias AgentCommand = NgtermCoreBindings.AgentCommand
/// Where a `/` menu entry comes from.
public typealias AgentCommandKind = NgtermCoreBindings.AgentCommandKind
/// What became of a chat action on a pane's agent.
public typealias AgentInputOutcome = NgtermCoreBindings.AgentInputOutcome
/// The user's answer to an ask: a button, typed text or the choices picked.
public typealias AskReply = NgtermCoreBindings.AskReply
/// A cell of a pane's grid: `x` its column, `y` its row in the viewport.
public typealias CellPoint = NgtermCoreBindings.CellPoint
/// A permission prompt a pane's agent waits on, from its hook payload.
public typealias ChatApproval = NgtermCoreBindings.ChatApproval
/// What a chat event says.
public typealias ChatBody = NgtermCoreBindings.ChatBody
/// One event of a pane's chat; a later one with the same id updates the item an earlier one made.
public typealias ChatEvent = NgtermCoreBindings.ChatEvent
/// What a chat notice reports.
public typealias ChatNoticeKind = NgtermCoreBindings.ChatNoticeKind
/// Claude's `AskUserQuestion`: its questions, and once answered, the answer the agent got.
public typealias ChatQuestion = NgtermCoreBindings.ChatQuestion
/// One question of an `AskUserQuestion`.
public typealias ChatQuestionItem = NgtermCoreBindings.ChatQuestionItem
/// One option of a question.
public typealias ChatQuestionOption = NgtermCoreBindings.ChatQuestionOption
/// What became of a pane's open chat, besides its events.
public typealias ChatState = NgtermCoreBindings.ChatState
/// A tool call of a chat, and once its result is written, how it went.
public typealias ChatTool = NgtermCoreBindings.ChatTool
/// What a tool does, for its glyph.
public typealias ChatToolKind = NgtermCoreBindings.ChatToolKind
/// Where a tool call or a question stands.
public typealias ChatToolStatus = NgtermCoreBindings.ChatToolStatus
/// How much of a limit is used, and when it resets.
public typealias ChatUsage = NgtermCoreBindings.ChatUsage
/// One representation of a clipboard value: its MIME type and bytes.
public typealias ClipboardPart = NgtermCoreBindings.ClipboardPart
/// The app's answer to a pane's clipboard read.
public typealias ClipboardReply = NgtermCoreBindings.ClipboardReply
/// How the app's write of a program's clipboard value ended.
public typealias ClipboardWriteStatus = NgtermCoreBindings.ClipboardWriteStatus
/// Why a key could not be made or imported.
public typealias KeyError = NgtermCoreBindings.KeyError
/// A private key in OpenSSH form, with the line for the machine's `authorized_keys` and its fingerprint.
public typealias KeyPair = NgtermCoreBindings.KeyPair
/// A machine the app knows, as it stores it and passes it in.
public typealias Machine = NgtermCoreBindings.Machine
/// A machine's connection.
public typealias MachineState = NgtermCoreBindings.MachineState
/// Why a core call failed.
public typealias MobileError = NgtermCoreBindings.MobileError
/// What the core tells the app on its own, through ``MobileCoreClient/events``.
public typealias MobileEvent = NgtermCoreBindings.MobileEvent
/// Which clipboard a pane's program reads or writes.
public typealias PaneClipboard = NgtermCoreBindings.PaneClipboard
/// The colors a pane's terminal starts from: the view's theme.
public typealias PaneColors = NgtermCoreBindings.PaneColors
/// How a shown pane's attach ended.
public typealias PaneEnd = NgtermCoreBindings.PaneEnd
/// Why a pane refused input.
public typealias PaneInputError = NgtermCoreBindings.PaneInputError
/// A key for a pane, with its modifiers and the text it produces.
public typealias PaneKey = NgtermCoreBindings.PaneKey
/// Whether a pane key is pressed, repeated or released.
public typealias PaneKeyAction = NgtermCoreBindings.PaneKeyAction
/// A pointer event for a pane's program.
public typealias PaneMouse = NgtermCoreBindings.PaneMouse
/// What a pointer event does: press, release or motion.
public typealias PaneMouseAction = NgtermCoreBindings.PaneMouseAction
/// A pointer event's button.
public typealias PaneMouseButton = NgtermCoreBindings.PaneMouseButton
/// What a machine's app offers beyond the protocol both apps speak, and the line the app shows about it.
public typealias PeerCapabilities = NgtermCoreBindings.PeerCapabilities
/// The app on a machine's far side: ngterm, or upstream agterm with fewer features.
public typealias PeerKind = NgtermCoreBindings.PeerKind
/// A shown pane: its machine, its session and its identity there.
public typealias PaneRef = NgtermCoreBindings.PaneRef
/// A shown pane's role at its session.
public typealias PaneRole = NgtermCoreBindings.PaneRole
/// What became of a paste.
public typealias PaneSendOutcome = NgtermCoreBindings.PaneSendOutcome
/// A pane's grid: its columns and rows and the cell's size in pixels.
public typealias PaneSize = NgtermCoreBindings.PaneSize
/// A wheel turn or scroll gesture over a pane.
public typealias PaneWheel = NgtermCoreBindings.PaneWheel
/// Where a wheel turn went.
public typealias PaneWheelOutcome = NgtermCoreBindings.PaneWheelOutcome
/// This app's role on an attached session.
public typealias PresentationRole = NgtermCoreBindings.PresentationRole
/// One question's answer: the option labels picked, and the text typed into its free-form option.
public typealias QuestionAnswer = NgtermCoreBindings.QuestionAnswer
/// An ask from an attached session.
public typealias RemoteAsk = NgtermCoreBindings.RemoteAsk
/// A button of an ask.
public typealias RemoteAskButton = NgtermCoreBindings.RemoteAskButton
/// A choice of an ask.
public typealias RemoteAskChoice = NgtermCoreBindings.RemoteAskChoice
/// The choices an ask offers.
public typealias RemoteAskChoices = NgtermCoreBindings.RemoteAskChoices
/// The text input an ask asks for.
public typealias RemoteAskInput = NgtermCoreBindings.RemoteAskInput
/// The HUD panel over an attached session.
public typealias RemoteHud = NgtermCoreBindings.RemoteHud
/// An attached session's panes and their arrangement.
public typealias RemoteLayout = NgtermCoreBindings.RemoteLayout
/// A notification from an attached session.
public typealias RemoteNotice = NgtermCoreBindings.RemoteNotice
/// A session as a machine's tree lists it.
public typealias RemoteSession = NgtermCoreBindings.RemoteSession
/// An attached session's agent status.
public typealias RemoteStatus = NgtermCoreBindings.RemoteStatus
/// An attached session's presentation stream's state.
public typealias StreamState = NgtermCoreBindings.StreamState
/// Why a ``KeyVault`` could not hand over a key.
public typealias VaultError = NgtermCoreBindings.VaultError
