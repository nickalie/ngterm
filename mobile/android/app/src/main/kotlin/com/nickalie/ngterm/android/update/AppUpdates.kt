package com.nickalie.ngterm.android.update

import android.content.Intent
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/** The intent action that opens the app on its update sheet (the update notification's tap). */
const val ACTION_SHOW_UPDATE = "com.nickalie.ngterm.android.SHOW_UPDATE"

/** The intent action that opens the app on its update sheet and starts the install (the notification's Install). */
const val ACTION_INSTALL_UPDATE = "com.nickalie.ngterm.android.INSTALL_UPDATE"

/** A newer release the app can install: its [version], the APK's [size] in bytes, and its release page. */
data class UpdateOffer(
    val version: String,
    val size: Long,
    val notesUrl: String,
)

/** Why a downloaded update was refused before Android saw it. */
enum class VerifyProblem {
    /** The release's checksum list isn't signed with ngterm's release key, or names another release. */
    Signature,

    /** The download doesn't match the checksum the signed list gives it. */
    Checksum,

    /** The APK isn't this app, or isn't newer than it. */
    Package,

    /** The APK is signed with another key than the installed app. */
    Signer,
}

/** What went wrong, by the step it went wrong at. */
sealed interface UpdateFailure {
    /** The releases couldn't be read, for [reason]. */
    data class Check(
        val reason: String,
    ) : UpdateFailure

    /** The download stopped, for [reason]. */
    data class Download(
        val reason: String,
    ) : UpdateFailure

    /** The download didn't verify. */
    data class Verify(
        val problem: VerifyProblem,
    ) : UpdateFailure

    /** Android's installer refused it, with its [reason] when it gave one. */
    data class Install(
        val reason: String?,
    ) : UpdateFailure
}

/** Where updating stands. */
sealed interface UpdateState {
    /** Not checked since the app started. */
    data object Idle : UpdateState

    /** Reading the releases. */
    data object Checking : UpdateState

    /** The installed version is the newest release. */
    data object Current : UpdateState

    /** [offer] is newer; [skipped] when the user said Not now to it, so it isn't notified again. */
    data class Available(
        val offer: UpdateOffer,
        val skipped: Boolean = false,
    ) : UpdateState

    /** [received] bytes of [offer] are in. */
    data class Downloading(
        val offer: UpdateOffer,
        val received: Long,
    ) : UpdateState

    /** The download is being checked against the release's signed checksums and the installed app's key. */
    data class Verifying(
        val offer: UpdateOffer,
    ) : UpdateState

    /** Handed to Android's installer; [waitingForYou] once it asks the user to confirm. */
    data class Installing(
        val offer: UpdateOffer,
        val waitingForYou: Boolean = false,
    ) : UpdateState

    /** [failure] stopped it; [offer] is the update it was installing, if it got that far. */
    data class Failed(
        val failure: UpdateFailure,
        val offer: UpdateOffer? = null,
    ) : UpdateState
}

/**
 * Updating the app from its GitHub releases, in the build that may (the GitHub one; Google Play's build has none, as
 * Play updates it). The screens show [state] and drive it; the update sheet is up while [prompt] holds.
 */
interface AppUpdates {
    /** The installed version. */
    val installed: String

    /** Where updating stands. */
    val state: StateFlow<UpdateState>

    /** Whether the app checks on its own: at start, and twice a day in the background. */
    val automatic: Flow<Boolean>

    /** Whether the update sheet should be up. */
    val prompt: StateFlow<Boolean>

    /** The app's screen came up: checks when [automatic], and keeps the background checks in line with it. */
    fun appStarted()

    /** Turns the automatic checks on or off. */
    fun setAutomatic(on: Boolean)

    /** Checks now. */
    fun check()

    /** Downloads, verifies and installs the offered update; brings Android's installer back while it waits. */
    fun install()

    /** Stops a download under way. */
    fun cancel()

    /** Not now: the offered version isn't notified again. */
    fun skip()

    /** An intent that reached the app's activity: an update notification's tap opens the sheet, its Install too. */
    fun opened(intent: Intent?)

    /** The update sheet was closed. */
    fun promptClosed()
}

/** The application, as what holds the app's updates: none in Google Play's build. */
interface UpdatesOwner {
    val updates: AppUpdates?
}
