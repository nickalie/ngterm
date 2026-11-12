package com.nickalie.ngterm.android.update

import android.app.Application

/**
 * Google Play's build has no updates of its own: Play updates it, and its policy forbids an app updating itself. It
 * takes the application as the GitHub build's twin does.
 */
@Suppress("FunctionOnlyReturningConstant", "UnusedParameter", "UNUSED_PARAMETER")
fun appUpdates(app: Application): AppUpdates? = null
