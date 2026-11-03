package com.nickalie.ngterm.android.ui

import androidx.window.core.layout.WindowSizeClass
import androidx.window.core.layout.computeWindowSizeClass
import org.junit.Assert.assertEquals
import org.junit.Test

/** The navigation's layout follows the window's width class, at Material's breakpoints. */
class NavLayoutTest {
    private fun layout(
        width: Float,
        height: Float = 900f,
    ) = NavLayout.of(WindowSizeClass.BREAKPOINTS_V2.computeWindowSizeClass(width, height))

    @Test
    fun aPhoneGetsTheBar() {
        assertEquals(NavLayout.Bar, layout(411f))
        assertEquals(NavLayout.Bar, layout(599f))
    }

    @Test
    fun aMediumWindowGetsTheRail() {
        assertEquals(NavLayout.Rail, layout(600f))
        assertEquals(NavLayout.Rail, layout(839f))
    }

    @Test
    fun anExpandedWindowAndWiderGetTheSidebar() {
        assertEquals(NavLayout.Sidebar, layout(840f))
        assertEquals(NavLayout.Sidebar, layout(1280f, 800f))
        assertEquals(NavLayout.Sidebar, layout(1920f, 1080f))
    }
}
