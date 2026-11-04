package com.nickalie.ngterm.android.ui.settings

import android.os.Build
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.data.TerminalTextSizeRange
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlin.math.roundToInt

/**
 * Settings: one column, 640dp at most, of the terminal (a live sample over its text size, font and colors, and
 * keeping the screen on), the appearance, the extra keys, the agents (the chat view), the notifications
 * ([notificationsOn]: whether the system lets the app post them), the machines trusted with the clipboard, the keys
 * ([keyWork]: a rotation under way or failed), and the loaded core's [coreVersion]. While the stores are read it
 * shows a spinner; when they can't be, why with Retry. [wallpaperColors] offers the wallpaper's colors (Android 12
 * and later).
 */
@Composable
fun SettingsScreen(
    state: SettingsUiState,
    coreVersion: String,
    notificationsOn: Boolean,
    keyWork: KeyWork,
    actions: SettingsActions,
    modifier: Modifier = Modifier,
    wallpaperColors: Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S,
) {
    when (state) {
        SettingsUiState.Loading -> {
            val reading = stringResource(R.string.settings_loading)
            Box(modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(Modifier.semantics { contentDescription = reading })
            }
        }

        is SettingsUiState.Failed -> {
            Box(modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) {
                ErrorState(
                    title = stringResource(R.string.settings_failed_title),
                    body = stringResource(R.string.settings_failed_body, state.reason),
                    action = PanelAction(stringResource(R.string.action_retry), onClick = actions.onRetry),
                )
            }
        }

        is SettingsUiState.Ready -> {
            ScreenFrame(modifier = modifier, title = stringResource(R.string.settings_title)) {
                Column {
                    TerminalSection(state.settings, actions.onChange)
                    AppearanceSection(state.settings, wallpaperColors, actions.onChange)
                    ExtraKeysSection(
                        state.settings.extraKeys,
                    ) { keys -> actions.onChange { it.copy(extraKeys = keys) } }
                    AgentsSection(state.settings.agentChat) { on -> actions.onChange { it.copy(agentChat = on) } }
                    NotificationsSection(state.alerts, notificationsOn, actions)
                    ClipboardSection(state.clipboard, actions.onRevokeClipboard)
                    KeysSection(state.keys, keyWork, actions)
                    AboutSection(coreVersion)
                }
            }
        }
    }
}

@Composable
private fun TerminalSection(
    settings: AppSettings,
    onChange: ((AppSettings) -> AppSettings) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_terminal), first = true) {
        val palette = settings.palette.pinned() ?: NgtermTheme.terminal
        val name = stringResource(paletteName(settings.palette))
        TerminalPreview(
            palette = palette,
            font = settings.font,
            textSize = settings.textSize,
            description = stringResource(R.string.settings_preview_description, name, settings.textSize),
            modifier = Modifier.padding(vertical = NgtermTheme.spacing.sm),
        )
        TextSize(settings.textSize, onSize = { size -> onChange { it.copy(textSize = size) } })
        Text(
            text = stringResource(R.string.settings_font),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.padding(top = NgtermTheme.spacing.md),
        )
        ChoiceGroup(
            options =
                listOf(
                    Choice(
                        FontChoice.JetBrainsMono,
                        stringResource(R.string.settings_font_jetbrains),
                        stringResource(R.string.settings_font_jetbrains_note),
                    ),
                    Choice(
                        FontChoice.SystemMono,
                        stringResource(R.string.settings_font_system),
                        stringResource(R.string.settings_font_system_note),
                    ),
                ),
            selected = settings.font,
            onSelect = { font -> onChange { it.copy(font = font) } },
        )
        Text(
            text = stringResource(R.string.settings_palette),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.padding(top = NgtermTheme.spacing.md),
        )
        PaletteChoices(settings.palette) { choice -> onChange { it.copy(palette = choice) } }
        SwitchRow(
            label = stringResource(R.string.settings_screen_on),
            note = stringResource(R.string.settings_screen_on_note),
            checked = settings.keepScreenOn,
            onCheckedChange = { on -> onChange { it.copy(keepScreenOn = on) } },
        )
    }
}

// the text size: its value in words, a slider of whole sp, and what it sets
@Composable
private fun TextSize(
    size: Int,
    onSize: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    // the slider moves freely under the finger and settles on its value's whole sp
    var dragged by remember(size) { mutableFloatStateOf(size.toFloat()) }
    val shown = dragged.roundToInt()
    val value = stringResource(R.string.settings_text_size_value, shown)
    val label = stringResource(R.string.settings_text_size)
    Column(modifier) {
        TextSizeHeader(label, value)
        TextSizeSlider(dragged, label, value, onDrag = { dragged = it }, onDone = { onSize(dragged.roundToInt()) })
        SettingsNote(stringResource(R.string.settings_text_size_note))
    }
}

@Composable
private fun TextSizeHeader(
    label: String,
    value: String,
) {
    Row(Modifier.fillMaxWidth().padding(top = NgtermTheme.spacing.sm), verticalAlignment = Alignment.CenterVertically) {
        Text(
            label,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f),
        )
        Text(value, style = NgtermTheme.mono.medium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun TextSizeSlider(
    position: Float,
    label: String,
    value: String,
    onDrag: (Float) -> Unit,
    onDone: () -> Unit,
) {
    val range = TerminalTextSizeRange
    Slider(
        value = position,
        onValueChange = onDrag,
        onValueChangeFinished = onDone,
        valueRange = range.first.toFloat()..range.last.toFloat(),
        steps = range.last - range.first - 1,
        // a quiet track: no verdigris (it means healthy) and no row of 23 ticks
        colors =
            SliderDefaults.colors(
                inactiveTrackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                activeTickColor = Color.Transparent,
                inactiveTickColor = Color.Transparent,
            ),
        modifier =
            Modifier.fillMaxWidth().semantics {
                contentDescription = label
                stateDescription = value
            },
    )
}

@Composable
private fun PaletteChoices(
    selected: PaletteChoice,
    onSelect: (PaletteChoice) -> Unit,
) {
    val notes =
        mapOf(
            PaletteChoice.Theme to R.string.settings_palette_theme_note,
            PaletteChoice.Copper to R.string.settings_palette_copper_note,
            PaletteChoice.Drafting to R.string.settings_palette_drafting_note,
            PaletteChoice.Ghostty to R.string.settings_palette_ghostty_note,
        )
    val theme = NgtermTheme.terminal
    ChoiceGroup(
        options =
            PaletteChoice.entries.map { choice ->
                Choice(choice, stringResource(paletteName(choice)), stringResource(notes.getValue(choice)))
            },
        selected = selected,
        onSelect = onSelect,
        trailing = { choice -> PaletteSwatch(choice.pinned() ?: theme) },
    )
}

private fun paletteName(choice: PaletteChoice): Int =
    when (choice) {
        PaletteChoice.Theme -> R.string.settings_palette_theme
        PaletteChoice.Copper -> R.string.settings_palette_copper
        PaletteChoice.Drafting -> R.string.settings_palette_drafting
        PaletteChoice.Ghostty -> R.string.settings_palette_ghostty
    }

@Composable
private fun AppearanceSection(
    settings: AppSettings,
    wallpaperColors: Boolean,
    onChange: ((AppSettings) -> AppSettings) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_appearance)) {
        ChoiceGroup(
            options =
                listOf(
                    Choice(
                        ThemeChoice.System,
                        stringResource(R.string.settings_theme_system),
                        stringResource(R.string.settings_theme_system_note),
                    ),
                    Choice(ThemeChoice.Light, stringResource(R.string.settings_theme_light)),
                    Choice(ThemeChoice.Dark, stringResource(R.string.settings_theme_dark)),
                ),
            selected = settings.theme,
            onSelect = { theme -> onChange { it.copy(theme = theme) } },
        )
        if (wallpaperColors) {
            SwitchRow(
                label = stringResource(R.string.settings_wallpaper),
                note = stringResource(R.string.settings_wallpaper_note),
                checked = settings.wallpaperColors,
                onCheckedChange = { on -> onChange { it.copy(wallpaperColors = on) } },
            )
        }
    }
}

@Composable
private fun ExtraKeysSection(
    selected: ExtraKeysChoice,
    onSelect: (ExtraKeysChoice) -> Unit,
) {
    SettingsSection(stringResource(R.string.settings_extra_keys)) {
        ChoiceGroup(
            options =
                listOf(
                    Choice(
                        ExtraKeysChoice.Fit,
                        stringResource(R.string.settings_keys_fit),
                        stringResource(R.string.settings_keys_fit_note),
                    ),
                    Choice(
                        ExtraKeysChoice.TwoLines,
                        stringResource(R.string.settings_keys_two),
                        stringResource(R.string.settings_keys_two_note),
                    ),
                    Choice(
                        ExtraKeysChoice.Hidden,
                        stringResource(R.string.settings_keys_hidden),
                        stringResource(R.string.settings_keys_hidden_note),
                    ),
                ),
            selected = selected,
            onSelect = onSelect,
        )
    }
}
