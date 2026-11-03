package com.nickalie.ngterm.android.platform

import android.app.Activity
import android.app.Application
import android.os.Bundle
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Whether any of the app's activities is started (on screen), counted from [application]'s activity callbacks. An
 * activity stopped for a configuration change (it is recreated at once) doesn't count as leaving the screen.
 */
class Foreground(
    application: Application,
) {
    private val shown = MutableStateFlow(false)
    private var started = 0

    /** Whether an activity of the app is started. */
    val visible: StateFlow<Boolean> = shown.asStateFlow()

    init {
        application.registerActivityLifecycleCallbacks(
            object : Application.ActivityLifecycleCallbacks {
                override fun onActivityStarted(activity: Activity) {
                    started += 1
                    shown.value = true
                }

                override fun onActivityStopped(activity: Activity) {
                    started = (started - 1).coerceAtLeast(0)
                    // a rotation, resize or theme switch stops the activity only to start it again at once
                    if (!activity.isChangingConfigurations) shown.value = started > 0
                }

                override fun onActivityCreated(
                    activity: Activity,
                    savedInstanceState: Bundle?,
                ) = Unit

                override fun onActivityResumed(activity: Activity) = Unit

                override fun onActivityPaused(activity: Activity) = Unit

                override fun onActivitySaveInstanceState(
                    activity: Activity,
                    outState: Bundle,
                ) = Unit

                override fun onActivityDestroyed(activity: Activity) = Unit
            },
        )
    }
}
