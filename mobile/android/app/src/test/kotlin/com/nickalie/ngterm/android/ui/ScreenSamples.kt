package com.nickalie.ngterm.android.ui

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import com.nickalie.ngterm.android.FAKE_FINGERPRINT
import com.nickalie.ngterm.android.FAKE_PUBLIC_KEY
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.MachinePeer
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.machines.MachineActions
import com.nickalie.ngterm.android.ui.machines.MachineItem
import com.nickalie.ngterm.android.ui.machines.MachinesScreen
import com.nickalie.ngterm.android.ui.machines.MachinesUiState
import com.nickalie.ngterm.android.ui.sessions.MachineSessions
import com.nickalie.ngterm.android.ui.sessions.SessionActions
import com.nickalie.ngterm.android.ui.sessions.SessionsScreen
import com.nickalie.ngterm.core.fake.TailscaleSignIn
import com.nickalie.ngterm.core.fake.fakePeerCapabilities
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.PeerKind

// The screens' content for the screenshot tests: real-looking machines and made-up keys (never a real one).

/** The clock the samples' reconnect countdown counts against. */
const val SAMPLE_NOW = 1_000_000L

val SampleKey = AppKey("k1", FAKE_PUBLIC_KEY, FAKE_FINGERPRINT, imported = false)

val SampleImportedKey =
    AppKey(
        "k2",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAnotherFakeKeyForScreenshotsOnly000000000000 nick@laptop",
        "SHA256:Zb3x9QeTf0LmN4kR7sVw2uYcHj6pDa1gEoI8nXy5Kq0",
        imported = true,
    )

// an agent's status as an origin lists it: its wire name, with a color and a shape only when the agent overrode them
private fun status(
    id: String,
    wire: String,
    color: String? = null,
    shape: String? = null,
    peer: PeerKind? = null,
) = fakeSession(id, peer = peer).copy(status = wire, statusColor = color, statusShape = shape)

/** One machine in each state the home shows. */
val SampleMachines =
    listOf(
        MachineItem(
            testMachine(id = "m1"),
            Link.Connected,
            listOf(
                status("s1", "active"),
                status("s2", "active"),
                status("s3", "blocked"),
                fakeSession("s4"),
            ),
        ),
        MachineItem(
            testMachine(id = "m2", label = "studio", host = "studio.tail0f3c.ts.net"),
            Link.Reconnecting(SAMPLE_NOW + 4_000),
            null,
        ),
        MachineItem(
            testMachine(id = "m3", label = "nas", host = "192.168.1.20").copy(user = "admin", port = 2222),
            Link.HostKeyChanged("SHA256:old", "SHA256:new"),
            null,
        ),
        MachineItem(testMachine(id = "m4", label = "pi", host = "pi.lan"), Link.HostKeyUnknown("SHA256:x"), null),
        MachineItem(
            testMachine(id = "m5", label = "lab-gpu", host = "lab-gpu.lan"),
            Link.Unreachable("lab-gpu.lan:22: connection refused"),
            null,
        ),
        MachineItem(testMachine(id = "m6", label = "", host = "scratch.lan"), Link.Offline, null),
    )

/** A machine whose connect waits for the user to sign in, as Tailscale SSH asks for a `check`. */
val SampleSignIn =
    MachineItem(
        testMachine(id = "m7", label = "studio", host = "studio.tail0f3c.ts.net"),
        Link.SignIn(TailscaleSignIn.message, TailscaleSignIn.url),
        null,
    )

val NoMachineActions = MachineActions({}, {}, {}, {}, {}, {}, {}, {})

/** [content] on the app's ground, as the first run's flow puts it. */
@Composable
fun Ground(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Surface(modifier = modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background, content = content)
}

/** [content] framed as the app frames a destination, its layout from the window. */
@Composable
fun InScaffold(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    var selected by remember { mutableStateOf(AppDestinations.first().key) }
    AdaptiveScaffold(AppDestinations, selected, { selected = it }, modifier, content = content)
}

/** The Machines home in [state], framed. */
@Composable
fun MachinesSample(
    state: MachinesUiState,
    modifier: Modifier = Modifier,
) {
    InScaffold(modifier) { MachinesScreen(state, NoMachineActions, now = { SAMPLE_NOW }) }
}

/** A machine's sessions: two agents at work (one attached), a companion, an idle shell, and one being ended. */
val SampleSessions =
    MachineSessions(
        machine = testMachine(),
        link = Link.Connected,
        sessions =
            listOf(
                status("s1", "active").copy(
                    name = "api-refactor",
                    context = "Split the billing service into its own crate",
                ),
                status("s3", "blocked").copy(
                    name = "docs-site",
                    context = "Rewrite the install guide",
                ),
                fakeSession("s4", name = "shell").copy(cwd = "/home/nick/src/ngterm"),
                fakeSession("s5", name = "old-experiment").copy(cwd = "/tmp/scratch"),
                // the core lists companions after every parent; the screen puts it under its own
                fakeSession("s2", name = "api-refactor review", parent = "s1"),
            ),
        failure = null,
        attached = setOf("s3"),
        offline = false,
        ending = setOf("s5"),
    )

/** What a Mac running agterm offers, as the core says. */
val SampleAgtermPeer = MachinePeer(PeerKind.AGTERM, fakePeerCapabilities(PeerKind.AGTERM))

/** A Mac running agterm, connected, with its sessions listed. */
val SampleAgtermMachine =
    MachineItem(
        testMachine(id = "m6", label = "studio", host = "studio.local").copy(user = "me"),
        Link.Connected,
        listOf(status("a1", "active", peer = PeerKind.AGTERM), fakeSession("a2", peer = PeerKind.AGTERM)),
        SampleAgtermPeer,
    )

/** That Mac's sessions screen. */
val SampleAgtermSessions =
    MachineSessions(
        machine = SampleAgtermMachine.machine,
        link = Link.Connected,
        sessions =
            listOf(
                status("a1", "active", peer = PeerKind.AGTERM).copy(
                    name = "claude",
                    context = "Port the settings sheet",
                ),
                fakeSession("a2", name = "shell", peer = PeerKind.AGTERM).copy(cwd = "/Users/me/src/app"),
            ),
        failure = null,
        attached = setOf("a1"),
        offline = false,
        peer = SampleAgtermPeer,
    )

val NoSessionActions = SessionActions({}, {}, {}, {}, {}, {}, {}, {}, {})

/** A machine's sessions in [state], framed. */
@Composable
fun SessionsSample(
    state: MachineSessions,
    modifier: Modifier = Modifier,
    starting: Boolean = false,
) {
    InScaffold(modifier) { SessionsScreen(state, NoSessionActions, now = { SAMPLE_NOW }, starting = starting) }
}
