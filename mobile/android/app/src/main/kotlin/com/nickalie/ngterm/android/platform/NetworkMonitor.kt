package com.nickalie.ngterm.android.platform

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Whether the device has a network that reaches the internet, in [online], and a nudge through [onChange] whenever
 * the default network comes, goes or changes (the core then retries what waits to reconnect).
 */
class NetworkMonitor(
    context: Context,
    private val onChange: () -> Unit,
) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val state = MutableStateFlow(isOnline())

    val online: StateFlow<Boolean> = state.asStateFlow()

    private fun isOnline(): Boolean =
        manager
            ?.getNetworkCapabilities(manager.activeNetwork)
            ?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true

    private val callback =
        object : ConnectivityManager.NetworkCallback() {
            // a new default network: what waits to reconnect goes now, over it
            override fun onAvailable(network: Network) {
                state.value = true
                onChange()
            }

            override fun onLost(network: Network) {
                state.value = isOnline()
            }

            override fun onCapabilitiesChanged(
                network: Network,
                capabilities: NetworkCapabilities,
            ) {
                val was = state.value
                state.value = capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                if (state.value && !was) onChange()
            }
        }

    /** Starts following the default network, for the app's lifetime. */
    fun start() {
        manager?.registerDefaultNetworkCallback(callback)
    }
}
