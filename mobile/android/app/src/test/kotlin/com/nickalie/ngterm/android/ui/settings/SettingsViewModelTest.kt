package com.nickalie.ngterm.android.ui.settings

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ENCRYPTED_FAKE_KEY
import com.nickalie.ngterm.android.FAKE_PASSPHRASE
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.onboarding.ImportProblem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SettingsViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val models = ViewModelStore()

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() {
        models.clear()
        test.close()
        Dispatchers.resetMain()
    }

    private fun model(): SettingsViewModel =
        ViewModelProvider(models, viewModelFactory { initializer { SettingsViewModel(test.graph) } })[
            SettingsViewModel::class.java,
        ]

    private fun SettingsViewModel.ready(predicate: (SettingsUiState.Ready) -> Boolean = { true }) =
        runBlocking { state.first { it is SettingsUiState.Ready && predicate(it) } as SettingsUiState.Ready }

    // what lands on a store's thread: wait for it
    private fun eventually(check: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!check()) {
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(10)
        }
    }

    @Test
    fun aChangedSettingIsStoredAndShown() {
        val model = model()
        assertEquals(13, model.ready().settings.textSize)
        model.change { it.copy(textSize = 16, palette = PaletteChoice.Ghostty, extraKeys = ExtraKeysChoice.TwoLines) }
        val ready = model.ready { it.settings.textSize == 16 }
        assertEquals(PaletteChoice.Ghostty, ready.settings.palette)
        assertEquals(ExtraKeysChoice.TwoLines, ready.settings.extraKeys)
        assertEquals(16, runBlocking { test.settings.settings.first() }.textSize)
    }

    @Test
    fun aNotificationKindTurnsOffAndOnAgain() {
        val model = model()
        assertEquals(AlertKind.entries.toSet(), model.ready().alerts)
        model.setAlert(AlertKind.Done, on = false)
        model.ready { AlertKind.Done !in it.alerts }
        assertFalse(AlertKind.Done in runBlocking { test.notificationPrefs.enabled.first() })
        model.setAlert(AlertKind.Done, on = true)
        model.ready { AlertKind.Done in it.alerts }
    }

    @Test
    fun aMachineTrustedWithTheClipboardIsListedUntilItAsksAgain() {
        test.seedMachine(testMachine())
        test.seedMachine(testMachine(id = "m2", label = "nas", host = "nas.lan"))
        runBlocking { test.trust.allow("m2") }
        // a machine removed since is not listed
        runBlocking { test.trust.allow("gone") }
        val model = model()
        val ready = model.ready { it.clipboard.isNotEmpty() }
        assertEquals(listOf(TrustedMachine("m2", "nas", "nick@nas.lan:22")), ready.clipboard)
        eventually { "m2" in test.clipboardReads.trusted.value }
        model.revokeClipboard("m2")
        model.ready { it.clipboard.isEmpty() }
        assertFalse("m2" in test.clipboardReads.trusted.value)
        assertEquals(setOf("gone"), runBlocking { test.trust.machines.first() })
    }

    @Test
    fun aRotatedKeyIsNewAndTheMachinesKeepTheOldOneUntilSwitched() {
        val old = test.seedKey()
        test.seedMachine(testMachine(keyId = old.id))
        test.seedMachine(testMachine(id = "m2", label = "nas", host = "nas.lan", keyId = old.id))
        val model = model()
        val before = model.ready { it.keys.size == 1 }
        assertEquals(listOf("build-01", "nas"), before.keys.single().usedBy)
        assertTrue(before.keys.single().everyMachine)
        model.rotate()
        eventually { model.newKey.value != null }
        val fresh = requireNotNull(model.newKey.value)
        val rotated = model.ready { it.keys.size == 2 }
        val item = rotated.keys.single { it.key.id == fresh }
        assertEquals(emptyList<String>(), item.usedBy)
        assertFalse(item.everyMachine)
        assertFalse(model.keyWork.value.rotating)
        // the private half is sealed in the vault, never in what Settings shows
        assertTrue(test.vault.has(fresh))
        model.newKeyShown()
        assertNull(model.newKey.value)
        model.useEverywhere(fresh)
        val switched = model.ready { state -> state.keys.single { it.key.id == fresh }.everyMachine }
        assertEquals(listOf("build-01", "nas"), switched.keys.single { it.key.id == fresh }.usedBy)
        assertEquals(emptyList<String>(), switched.keys.single { it.key.id == old.id }.usedBy)
    }

    @Test
    fun anImportSaysWhatWasWrongThenLandsAsANewKey() {
        test.seedKey()
        val model = model()
        model.startImport()
        model.import("  \n".toByteArray(), null)
        assertEquals(ImportProblem.Empty, model.keyWork.value.importProblem)
        model.import(ENCRYPTED_FAKE_KEY.toByteArray(), "wrong")
        eventually { model.keyWork.value.importProblem == ImportProblem.WrongPassphrase }
        model.import(ENCRYPTED_FAKE_KEY.toByteArray(), FAKE_PASSPHRASE)
        eventually { model.newKey.value != null }
        assertNull(model.keyWork.value.importProblem)
        assertFalse(model.keyWork.value.importing)
        val imported = model.ready { it.keys.size == 2 }.keys.single { it.key.id == model.newKey.value }
        assertTrue(imported.key.imported)
        // opening the import again starts clean
        model.unreadable()
        model.startImport()
        assertNull(model.keyWork.value.importProblem)
    }
}
