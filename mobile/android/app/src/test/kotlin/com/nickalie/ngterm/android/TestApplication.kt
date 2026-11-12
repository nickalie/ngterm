package com.nickalie.ngterm.android

import android.app.Application
import com.nickalie.ngterm.android.update.AppUpdates
import com.nickalie.ngterm.android.update.UpdatesOwner

/**
 * The application of the tests that reach the app's graph from outside the screens (a notification's action, the
 * foreground service, the updates' receivers and worker): the [graph] is the test's own ([TestGraph]), set before it
 * is reached, and so are its [updates].
 */
class TestApplication :
    Application(),
    GraphOwner,
    UpdatesOwner {
    lateinit var testGraph: AppGraph

    override val graph: AppGraph get() = testGraph

    override var updates: AppUpdates? = null
}
