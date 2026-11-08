package com.nickalie.ngterm.android

import android.app.Activity
import android.net.ConnectivityManager
import android.os.Build
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.platform.TestAndroidKeystore
import com.nickalie.ngterm.android.service.LinkService
import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.KeyVault
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.model.FrameworkMethod
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.internal.bytecode.InstrumentationConfiguration
import org.robolectric.util.ReflectionHelpers

/** Startup builds real stores and platform services; only the unavailable JNI/Keystore services are substituted. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(ApplicationStartupRunner::class)
@Config(
    application = NgtermApplication::class,
    shadows = [StartupFunctionsShadow::class],
    instrumentedPackages = ["com.nickalie.ngterm.core.ffi"],
)
class NgtermApplicationTest {
    private val app = ApplicationProvider.getApplicationContext<NgtermApplication>()

    @Before fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        TestAndroidKeystore.install()
    }

    @After fun tearDown() {
        TestAndroidKeystore.remove()
        Dispatchers.resetMain()
    }

    private fun <T> stored(read: suspend () -> T): T = runBlocking { withTimeout(10_000) { read() } }

    @Test
    fun theProcessBuildsOneGraphWithWorkingStoresAndANetworkRetryCallback() =
        stored {
            val graph = app.graph
            assertSame(graph, app.graph)
            graph.pins.pin("m1", "SHA256:test")
            assertEquals("SHA256:test", graph.pins.pinned("m1"))
            assertEquals(AppSettings(), graph.settings.settings.first())
            graph.settings.update { it.copy(theme = ThemeChoice.Dark) }
            assertEquals(
                ThemeChoice.Dark,
                graph.settings.settings
                    .first()
                    .theme,
            )
            assertTrue(
                graph.machines.machines
                    .first()
                    .isEmpty(),
            )
            assertTrue(
                graph.keys.keys
                    .first()
                    .isEmpty(),
            )
            assertTrue(
                graph.notificationPrefs.enabled
                    .first()
                    .isNotEmpty(),
            )
            val manager = app.getSystemService(ConnectivityManager::class.java)
            assertEquals(1, shadowOf(manager).networkCallbacks.size)
            shadowOf(manager).networkCallbacks.single().onAvailable(requireNotNull(manager.activeNetwork))
            val core = (graph.connections.core as StartupMobileCore).fake
            assertTrue(core.calls.any { it.method == "networkChanged" })
        }

    @Test
    fun foregroundResumesTheCoreAndAnAttachedSessionStartsTheLinkService() {
        val graph = app.graph
        val core = (graph.connections.core as StartupMobileCore).fake
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(core.calls.any { it.method == "suspend" })
        val activity = Robolectric.buildActivity(Activity::class.java).setup()
        try {
            shadowOf(Looper.getMainLooper()).idle()
            assertTrue(core.calls.any { it.method == "resume" })
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            graph.connections.connect(testMachine().toCore())
            shadowOf(Looper.getMainLooper()).idle()
            stored { graph.connections.attach("m1", "s1") }
            shadowOf(Looper.getMainLooper()).idle()
            assertEquals(LinkService::class.java.name, shadowOf(app).nextStartedService.component?.className)
        } finally {
            activity.pause().stop().destroy()
        }
    }

    @Test
    fun theFirstKeyHasASafeDeviceCommentAndItsPrivateHalfCanBeOpened() =
        stored {
            ReflectionHelpers.setStaticField(Build::class.java, "MODEL", " Pixel / 9 ! ")
            val key = app.graph.keys.generate()
            assertEquals("ssh-ed25519 test ngterm@Pixel-9", key.authorizedKey)
            val vault = (app.graph.connections.core as StartupMobileCore).vault
            assertEquals("test private bytes", String(vault.load(key.id)))
            assertEquals(
                key,
                app.graph.keys.keys
                    .first()
                    .single(),
            )
        }

    @Test
    fun aDeviceWithNoUsableModelGetsTheAndroidComment() =
        stored {
            ReflectionHelpers.setStaticField(Build::class.java, "MODEL", "!? ")
            assertEquals(
                "ssh-ed25519 test ngterm@android",
                app.graph.keys
                    .generate()
                    .authorizedKey,
            )
        }

    @Test
    fun aNullDeviceModelGetsTheAndroidComment() =
        stored {
            ReflectionHelpers.setStaticField(Build::class.java, "MODEL", null)
            assertEquals(
                "ssh-ed25519 test ngterm@android",
                app.graph.keys
                    .generate()
                    .authorizedKey,
            )
        }
}

/** Substitute the native core at its FFI constructor before Kotlin evaluates UniFFI's native delegation arguments. */
class ApplicationStartupRunner(
    testClass: Class<*>,
) : RobolectricTestRunner(testClass) {
    override fun createClassLoaderConfig(method: FrameworkMethod): InstrumentationConfiguration =
        InstrumentationConfiguration
            .Builder(super.createClassLoaderConfig(method))
            .addInstrumentedPackage("com.nickalie.ngterm.android.NgtermApplication")
            .addClassNameTranslation("com.nickalie.ngterm.core.ffi.MobileCore", StartupMobileCore::class.java.name)
            .build()
}

/** The existing core fake with the native constructor's signature; every app service remains real. */
class StartupMobileCore private constructor(
    val fake: FakeMobileCore,
    val vault: KeyVault,
) : MobileCoreInterface by fake {
    constructor(sink: EventSink, vault: KeyVault, hosts: HostKeyStore) : this(FakeMobileCore(sink, hosts), vault)
}
