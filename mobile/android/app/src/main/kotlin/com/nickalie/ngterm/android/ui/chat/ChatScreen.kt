package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.components.SectionAction
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.QuestionAnswer

/**
 * A pane's open [chat], live, read from [machine]; [onShowTerminal] shows the pane's terminal instead. Its prompt field
 * takes the input focus through [focusRequester]; its draft is kept while the app is put away ([KeepDraft]). Back
 * closes its `/` menu only while it is the [selected] pane.
 */
@Composable
fun ChatPane(
    chat: ChatViewModel,
    machine: String,
    onShowTerminal: () -> Unit,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester = remember { FocusRequester() },
    selected: Boolean = true,
) {
    val state by chat.state.collectAsStateWithLifecycle()
    val resolver = LocalContext.current.contentResolver
    val actions =
        remember(chat, onShowTerminal, resolver) {
            ChatActions(
                onLoadOlder = chat::loadOlder,
                onAnswerApproval = chat::answerApproval,
                onAnswerQuestion = chat::answerQuestion,
                onProblemShown = chat::problemShown,
                onShowTerminal = onShowTerminal,
                onSend = chat::send,
                onStop = chat::stop,
                onEndShown = chat::endShown,
                onPick = { uri ->
                    val picked = pickedFile(resolver, uri)
                    chat.attach(picked.name, picked.size) { readPicked(resolver, uri) }
                },
                onCancelUpload = chat::cancelUpload,
                onUploadShown = chat::uploadShown,
            )
        }
    KeepDraft(chat.pane, chat.composer, state.conversation?.conversationId, chat::restoreDraft)
    ChatScreen(state, machine, actions, modifier, chat.composer, focusRequester, selected)
}

/**
 * A pane's chat ([state]), read from [machine]: its rows ([ChatList]), or what stands in for them (the backlog on its
 * way, nothing said yet, the chat ended), a strip over the rows while its link is back on its way, pinned at its foot
 * what the agent still has running ([ActivityStrip], unless a card waits), the last action's problem and the card the
 * agent waits on (an approval before a question), and under them the
 * [composer] (none: the chat only reads), whose `/` menu opens over the foot of the rows, under them the activity line
 * and its Stop, or over the card while one waits. The prompt field
 * takes the input focus through [focusRequester]; Back closes the menu only in the [selected] pane. TalkBack names it
 * "Chat with" the agent.
 */
@Composable
fun ChatScreen(
    state: ChatUiState,
    machine: String,
    actions: ChatActions,
    modifier: Modifier = Modifier,
    composer: ChatComposer? = null,
    focusRequester: FocusRequester = remember { FocusRequester() },
    selected: Boolean = true,
) {
    val agent = state.conversation?.agent
    val name = stringResource(agentName(agent))
    val description = stringResource(R.string.chat_description, name)
    BoxWithConstraints(
        modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .semantics { contentDescription = description },
    ) {
        val cardHeight = maxHeight * CARD_SHARE
        val writing = composer?.takeIf { state.phase != ChatPhase.Ended }
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                Column(Modifier.fillMaxSize()) {
                    state.account?.takeIf { state.phase != ChatPhase.Ended }?.let { account ->
                        AccountLine(
                            account,
                            Modifier
                                .align(Alignment.CenterHorizontally)
                                .widthIn(max = NgtermTheme.spacing.readableWidth)
                                .padding(horizontal = NgtermTheme.spacing.lg),
                        )
                    }
                    Box(Modifier.weight(1f).fillMaxWidth()) {
                        ChatContent(state, machine, name, actions)
                        // over the rows alone while no card waits, so the activity line's Stop stays in reach
                        if (!state.awaitsCard) {
                            writing?.let {
                                CommandMenu(
                                    it,
                                    agent,
                                    focusRequester,
                                    Modifier.align(Alignment.BottomCenter),
                                )
                            }
                        }
                        if (state.phase == ChatPhase.Reconnecting) {
                            ReconnectingStrip(
                                machine,
                                Modifier
                                    .align(Alignment.TopCenter)
                                    .padding(NgtermTheme.spacing.sm)
                                    .widthIn(max = NgtermTheme.spacing.readableWidth),
                            )
                        }
                    }
                    if (state.phase != ChatPhase.Ended) Pinned(state, actions, cardHeight)
                }
                // over the card too while one waits: it has no Stop, and the menu rises from the composer
                if (state.awaitsCard) {
                    writing?.let { CommandMenu(it, agent, focusRequester, Modifier.align(Alignment.BottomCenter)) }
                }
            }
            writing?.let { Composer(it, state, actions, focusRequester, selected = selected) }
        }
    }
}

// the `/` menu of [writing] while it is open, over what is above the composer
@Composable
private fun CommandMenu(
    writing: ChatComposer,
    agent: String?,
    focusRequester: FocusRequester,
    modifier: Modifier = Modifier,
) {
    val menu = writing.menu ?: return
    SlashCommandMenu(
        menu = menu,
        agent = agent,
        onPick = { command ->
            writing.pick(command)
            // the field keeps the input focus, and the keyboard with it
            focusRequester.requestFocus()
        },
        modifier =
            modifier
                .widthIn(max = NgtermTheme.spacing.readableWidth)
                .padding(horizontal = NgtermTheme.spacing.sm, vertical = NgtermTheme.spacing.xs),
    )
}

// the most of the chat's height a card takes before it scrolls inside: the rows above it stay in sight
private const val CARD_SHARE = 0.6f

@Composable
private fun ChatContent(
    state: ChatUiState,
    machine: String,
    name: String,
    actions: ChatActions,
    modifier: Modifier = Modifier,
) {
    val waiting = state.phase == ChatPhase.Loading || state.phase == ChatPhase.Reconnecting
    val terminal = PanelAction(stringResource(R.string.session_show_terminal), onClick = actions.onShowTerminal)
    when {
        state.phase == ChatPhase.Ended -> {
            Centered(modifier) {
                EmptyState(
                    icon = R.drawable.ic_chat,
                    title = stringResource(R.string.chat_ended_title),
                    body = stringResource(R.string.chat_ended_body, name, machine),
                    action = terminal,
                )
            }
        }

        state.log.isEmpty && !state.log.switched && waiting -> {
            Centered(modifier) { ChatLoading(machine) }
        }

        state.log.isEmpty && !state.log.switched -> {
            Centered(modifier) {
                EmptyState(
                    icon = R.drawable.ic_chat,
                    title = stringResource(R.string.chat_empty_title),
                    body = stringResource(R.string.chat_empty_body, name),
                    action = terminal,
                )
            }
        }

        else -> {
            ChatList(state, actions, modifier)
        }
    }
}

@Composable
private fun Centered(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) { content() }
}

// what waits at the chat's foot: what still runs (while no card waits), the last action's problem, then the card
// (no taller than [cardHeight])
@Composable
private fun Pinned(
    state: ChatUiState,
    actions: ChatActions,
    cardHeight: Dp,
    modifier: Modifier = Modifier,
) {
    val agent = state.conversation?.agent
    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.BottomCenter) {
        Column(
            modifier = Modifier.widthIn(max = NgtermTheme.spacing.readableWidth).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        ) {
            if (!state.awaitsCard) {
                ActivityStrip(
                    working = state.working,
                    running = state.log.running,
                    agent = stringResource(agentName(agent)),
                    modifier = Modifier.padding(horizontal = NgtermTheme.spacing.sm).fillMaxWidth(),
                    onStop = actions.onStop.takeIf { state.running },
                    stopEnabled = !state.busy,
                )
            }
            state.problem?.let { problem ->
                ProblemBanner(problem, actions, Modifier.padding(NgtermTheme.spacing.sm))
            }
            val card = Modifier.heightIn(max = cardHeight)
            val approval = state.approval
            val question = state.question
            when {
                // a tap answers the card it was drawn on, whatever replaced it before the tap landed
                approval != null -> {
                    val onAnswer: (String) -> Unit = { label -> actions.onAnswerApproval(approval.id, label) }
                    ApprovalCardView(approval, agent, state.busy, onAnswer, card)
                }

                question != null -> {
                    val onAnswer: (List<QuestionAnswer>) -> Unit = { answers ->
                        actions.onAnswerQuestion(question.id, answers)
                    }
                    QuestionCardView(question, agent, state.busy, onAnswer, card)
                }
            }
        }
    }
}

// why the last action did nothing, or not all it meant to, and Answer in terminal when the terminal is the way on
@Composable
private fun ProblemBanner(
    problem: ChatProblem,
    actions: ChatActions,
    modifier: Modifier = Modifier,
) {
    val terminal =
        if (problem.offersTerminal) {
            SectionAction(stringResource(R.string.chat_answer_in_terminal), actions.onShowTerminal)
        } else {
            null
        }
    Banner(
        tone = problem.tone,
        message = problem.message(),
        modifier = modifier,
        action = terminal,
        onDismiss = actions.onProblemShown,
    )
}

// a problem the user can act on is Needs you; one that only reports is Failed
private val ChatProblem.tone: StatusTone
    get() =
        when (this) {
            ChatProblem.NotOnScreen, ChatProblem.DialogOpen, ChatProblem.Unconfirmed -> StatusTone.Attention
            else -> StatusTone.Error
        }

@Composable
private fun ChatProblem.message(): String =
    when (this) {
        ChatProblem.NotOnScreen -> stringResource(R.string.chat_problem_not_on_screen)
        ChatProblem.DialogOpen -> stringResource(R.string.chat_problem_dialog_open)
        ChatProblem.LeadRefused -> stringResource(R.string.chat_problem_lead_refused)
        ChatProblem.Unconfirmed -> stringResource(R.string.chat_problem_unconfirmed)
        ChatProblem.NotReading -> stringResource(R.string.chat_problem_not_reading)
        ChatProblem.Closed -> stringResource(R.string.chat_problem_closed)
        is ChatProblem.Refused -> stringResource(R.string.chat_problem_refused, reason)
    }
