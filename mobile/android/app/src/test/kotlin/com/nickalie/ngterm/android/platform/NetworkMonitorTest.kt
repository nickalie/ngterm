package com.nickalie.ngterm.android.platform

import android.content.Context
import android.content.ContextWrapper
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowNetwork
import org.robolectric.shadows.ShadowNetworkCapabilities

/** A retry follows a new default network or restored internet, without repeated nudges for the same network. */
@RunWith(AndroidJUnit4::class)
class NetworkMonitorTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val system = shadowOf(manager)

    private fun capabilities(internet: Boolean): NetworkCapabilities =
        ShadowNetworkCapabilities.newInstance().also {
            if (internet) shadowOf(it).addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        }

    @Test
    fun initialStateComesFromTheActiveNetworksInternetCapability() {
        val network = manager.activeNetwork
        system.setNetworkCapabilities(network, capabilities(true))
        assertTrue(NetworkMonitor(context) {}.online.value)
        system.setNetworkCapabilities(network, capabilities(false))
        assertFalse(NetworkMonitor(context) {}.online.value)
        system.setNetworkCapabilities(network, null)
        assertFalse(NetworkMonitor(context) {}.online.value)
        system.clearAllNetworks()
        assertFalse(NetworkMonitor(context) {}.online.value)
    }

    @Test
    fun availableAndRestoredNetworksNudgeRetriesAndLossUsesTheRemainingNetwork() {
        system.setNetworkCapabilities(manager.activeNetwork, capabilities(false))
        var retries = 0
        val monitor = NetworkMonitor(context) { retries++ }
        monitor.start()
        val callback = system.networkCallbacks.single()
        val network = ShadowNetwork.newInstance(42)
        system.setNetworkCapabilities(network, capabilities(true))
        callback.onAvailable(network)
        assertTrue(monitor.online.value)
        assertEquals(1, retries)
        callback.onCapabilitiesChanged(network, capabilities(true))
        assertEquals(1, retries)
        callback.onCapabilitiesChanged(network, capabilities(false))
        assertFalse(monitor.online.value)
        callback.onCapabilitiesChanged(network, capabilities(false))
        assertEquals(1, retries)
        callback.onCapabilitiesChanged(network, capabilities(true))
        assertTrue(monitor.online.value)
        assertEquals(2, retries)
        system.setNetworkCapabilities(manager.activeNetwork, capabilities(true))
        callback.onLost(network)
        assertTrue(monitor.online.value)
        system.clearAllNetworks()
        callback.onLost(network)
        assertFalse(monitor.online.value)
        assertEquals(2, retries)
    }

    @Test
    fun aDeviceWithoutConnectivityServiceStaysOffline() {
        val absent =
            object : ContextWrapper(context) {
                override fun getSystemService(name: String): Any? =
                    if (name == Context.CONNECTIVITY_SERVICE) null else super.getSystemService(name)
            }
        var retries = 0
        val monitor = NetworkMonitor(absent) { retries++ }
        monitor.start()
        assertFalse(monitor.online.value)
        assertEquals(0, retries)
    }
}
