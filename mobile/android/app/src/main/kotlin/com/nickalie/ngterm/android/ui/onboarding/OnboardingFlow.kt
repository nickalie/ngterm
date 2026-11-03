package com.nickalie.ngterm.android.ui.onboarding

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * The first run, full screen and without the app's navigation (there is nowhere to go without a key): the welcome,
 * the import, and the public key once there is one. Back from the import returns to the welcome.
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
            )
    }
}
