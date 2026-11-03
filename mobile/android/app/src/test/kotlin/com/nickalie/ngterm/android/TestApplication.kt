package com.nickalie.ngterm.android

import android.app.Application

/**
 * The application of the tests that reach the app's graph from outside the screens (a notification's action, the
 * foreground service): the [graph] is the test's own ([TestGraph]), set before it is reached.
 */
class TestApplication :
    Application(),
    GraphOwner {
    lateinit var testGraph: AppGraph

    override val graph: AppGraph get() = testGraph
}
