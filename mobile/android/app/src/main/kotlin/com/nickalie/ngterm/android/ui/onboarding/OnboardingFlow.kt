package com.nickalie.ngterm.android.ui.onboarding

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * The first run, full screen and without the app's navigation (there is nowhere to go without a key): the welcome,
 * the import, and the public key once there is one, or why the app's keys couldn't be read, with a retry. Back from the
 * import returns to the welcome.
 */
@Composable
fun OnboardingFlow(
    step: OnboardingStep,
    actions: OnboardingActions,
    modifier: Modifier = Modifier,
) {
    BackHandler(enabled = step is OnboardingStep.Import, onBack = actions.onBack)
    val motion = NgtermTheme.motion
    Surface(modifier = modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        AnimatedContent(
            targetState = step,
            contentKey = { it.javaClass },
            transitionSpec = { fadeIn(motion.standard()) togetherWith fadeOut(motion.quick()) },
            label = "onboarding",
        ) { current ->
            when (current) {
                is OnboardingStep.Welcome -> {
                    WelcomeScreen(
                        busy = current.busy,
                        failure = current.failure,
                        onCreate = actions.onCreate,
                        onImport = actions.onStartImport,
                    )
                }

                is OnboardingStep.Import -> {
                    ImportKeyScreen(
                        busy = current.busy,
                        problem = current.problem,
                        onImport = actions.onImport,
                        onUnreadable = actions.onUnreadable,
                        onBack = actions.onBack,
                    )
                }

                is OnboardingStep.ReadFailed -> {
                    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
                        ErrorState(
                            title = stringResource(R.string.onboarding_read_failed_title),
                            body = stringResource(R.string.onboarding_read_failed_body, current.reason),
                            action = PanelAction(stringResource(R.string.action_retry), onClick = actions.onRetry),
                        )
                    }
                }

                is OnboardingStep.KeyReady -> {
                    PublicKeyScreen(key = current.key, onDone = actions.onFinish)
                }

                OnboardingStep.Loading, OnboardingStep.Done -> {
                    Box(Modifier.fillMaxSize())
                }
            }
        }
    }
}

/** What the first run's screens do, as [OnboardingViewModel] does it. */
data class OnboardingActions(
    val onCreate: () -> Unit,
    val onStartImport: () -> Unit,
    val onImport: (ByteArray, String?) -> Unit,
    val onUnreadable: () -> Unit,
    val onBack: () -> Unit,
    val onFinish: () -> Unit,
    val onRetry: () -> Unit,
) {
    companion object {
        /** The actions of [viewModel]. */
        fun of(viewModel: OnboardingViewModel) =
            OnboardingActions(
                onCreate = viewModel::create,
                onStartImport = viewModel::startImport,
                onImport = viewModel::import,
                onUnreadable = viewModel::unreadable,
                onBack = viewModel::back,
                onFinish = viewModel::finish,
                onRetry = viewModel::retry,
            )
    }
}
