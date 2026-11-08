"""`config.reload`'s `count`: one diagnostic per line of the agterm-scoped `<state>/config/ghostty.conf` Ghostty's
config loader refuses, none for a line it takes, by the kind of the key's value.

Each case appends its lines to the instance's own config, reloads and puts the file back. The counts are libghostty's
own (its `cli/args.zig` and the config types' `parseCLI`, `Binding.zig` for `keybind`), as the Rust build's goldens
recorded them against a linked libghostty (crates/ngterm-core/src/ghostty_config/counts_tests.rs); the cases keep to
keys and values every Ghostty the reference and the Rust build carry agree on. The module shares one instance."""

import pytest
from agterm_conformance.match import assert_ok

# (id, [(line, diagnostics)]): each line alone after the instance's own config, its count from `config.reload`.
CASES = [
    (
        "unknown-key",
        [
            ("no-such-key = 1", 1),
            ('"quoted key" = 1', 1),
        ],
    ),
    (
        "bool",
        [
            ("mouse-hide-while-typing = T", 0),
            ("mouse-hide-while-typing", 0),
            ("mouse-hide-while-typing = yes", 1),
            ("cursor-invert-fg-bg = true", 0),
            ("cursor-invert-fg-bg = maybe", 1),
            ("bold-is-bright", 0),
        ],
    ),
    (
        "enum",
        [
            ("macos-titlebar-style = tabs", 0),
            ("macos-titlebar-style = nope", 1),
            ("cursor-style = blcok", 1),
            ("cursor-style", 1),
            ("auto-update = sometimes", 1),
            ("gtk-tabs-location = hidden", 0),
            ("gtk-tabs-location = left", 1),
            ("window-decoration", 0),
            ("window-decoration = false", 0),
            ("window-decoration = maybe", 1),
        ],
    ),
    (
        "integer",
        [
            ("abnormal-command-exit-runtime = 0x10", 0),
            ("abnormal-command-exit-runtime = 1_000", 0),
            ("abnormal-command-exit-runtime = -1", 1),
            ("abnormal-command-exit-runtime = 4294967296", 1),
            ("window-position-x = -32768", 0),
            ("click-repeat-interval = 0x3e8", 0),
            ("scrollback-limit = 1000", 0),
            ("scrollback-limit = lots", 1),
        ],
    ),
    (
        "float",
        [
            ("background-opacity = 0.5", 0),
            ("background-opacity = 1_0.5", 0),
            ("background-opacity = 0x1p-1", 0),
            ("background-opacity = .5", 0),
            ("background-opacity = 5.", 0),
            ("background-opacity = inf", 0),
            ("background-opacity = abc", 1),
            ("background-opacity = 1__0", 1),
            ("background-opacity = 1e", 1),
            ("background-opacity = .", 1),
        ],
    ),
    (
        "color",
        [
            ("background = #123", 0),
            ("background = #111222333", 0),
            ("background = Dark Slate Gray", 0),
            ("background = DARKSLATEGRAY", 0),
            ("background = a1b2c3", 0),
            ("background = rgb:f/ff/fff", 0),
            ("background = rgbi:0.5/1/0", 0),
            ("background = #12", 1),
            ("background = rgb:1/2", 1),
            ("background = rgbi:2/0/0", 1),
            ("background = notacolor", 1),
            ("background = aé", 1),
            ("cursor-color = cell-foreground", 0),
            ("selection-background = cell-backgrounds", 1),
            ("bold-color = bright", 0),
            ("bold-color = dull", 1),
        ],
    ),
    (
        "palette",
        [
            ("palette = 0xff=red", 0),
            ("palette = 256=#000000", 1),
            ("palette = 3", 1),
            ("palette = 1=éa", 1),
        ],
    ),
    (
        "string",
        [
            ('selection-word-chars = " \\t;\\x41\\u{2502}"', 0),
            ('selection-word-chars = "\\q"', 1),
            ('selection-word-chars = "\\x4"', 1),
            ("title", 1),
            ("term =", 0),
        ],
    ),
    (
        "duration",
        [
            ("notify-on-command-finish-after = 1h 30m", 0),
            ("notify-on-command-finish-after = 0", 0),
            ("notify-on-command-finish-after = 1m1ms", 0),
            ("notify-on-command-finish-after = 5µs", 0),
            ("notify-on-command-finish-after = 5", 1),
            ("notify-on-command-finish-after = 10x", 1),
        ],
    ),
    (
        "padding",
        [
            ("window-padding-x = 4,8", 0),
            ("window-padding-x = 4,", 1),
            ("window-padding-x = -1", 1),
        ],
    ),
    (
        "flags",
        [
            ("shell-integration-features = no-cursor,sudo", 0),
            ("shell-integration-features = true", 0),
            ("shell-integration-features = cursor,", 1),
            ("shell-integration-features = bogus", 1),
            ("scrollback-limit-bytes = unlimited", 0),
            ("scrollback-limit-bytes = lots", 1),
        ],
    ),
    (
        "theme",
        # parse-time refusals; one Ghostty refuses as it loads the theme is test_reload_counts_a_theme_refused_as_it_loads
        [
            ("theme = light:Only", 1),
            ('theme = light:"\\q",dark:x', 1),
            ("theme = light:a\\qb,dark:x", 1),
            ('theme = light:"\\x5",dark:x', 1),
        ],
    ),
    (
        "config-file",
        [
            ("config-file = ?absent.conf", 0),
            ("config-file = absent.conf", 1),
            ("config-file =", 0),
        ],
    ),
    (
        "keybind",
        [
            ("keybind = ctrl+a=select_all", 0),
            ("keybind = ctrl+shift+key_c=copy_to_clipboard", 0),
            ("keybind = ctrl+KeyC=copy_to_clipboard:vt", 0),
            ("keybind = ArrowUp=scroll_page_up", 0),
            ("keybind = unconsumed:ctrl+a=text:x", 0),
            ("keybind = performable:all:ctrl+a=reset", 0),
            ("keybind = ctrl+a>b=new_window", 0),
            ("keybind = ctrl+==increase_font_size:1", 0),
            ("keybind = ctrl++=increase_font_size:2.5", 0),
            ("keybind = +=text:plus", 0),
            ("keybind = catch_all=ignore", 0),
            ("keybind = physical:one=goto_tab:1", 0),
            ("keybind = kp_enter=csi:A", 0),
            ("keybind = grave_accent=esc:x", 0),
            ("keybind = cmd+opt+control+a=ignore", 0),
            ("keybind = ф=ignore", 0),
            ("keybind = ctrl+a=goto_split:top", 0),
            ("keybind = ctrl+a=resize_split:up,10", 0),
            ("keybind = ctrl+a=write_screen_file:open,html", 0),
            ("keybind = ctrl+a=jump_to_prompt:-1", 0),
            ("keybind = ctrl+a=scroll_page_fractional:0.5", 0),
            ("keybind = ctrl+a=set_font_size:1e3", 0),
            ("keybind = clear", 0),
            ("keybind", 1),
            ("keybind = garbage", 1),
            ("keybind = a=nope", 1),
            ("keybind = a=", 1),
            ("keybind = =ignore", 1),
            ("keybind = ctrl+ctrl+a=ignore", 1),
            ("keybind = a+b=ignore", 1),
            ("keybind = foo=ignore", 1),
            ("keybind = all:all:a=ignore", 1),
            ("keybind = global:a>b=ignore", 1),
            ("keybind = a=select_all:x", 1),
            ("keybind = a=text", 1),
            ("keybind = a=increase_font_size:big", 1),
            ("keybind = a=goto_split:sideways", 1),
            ("keybind = a=resize_split:up", 1),
            ("keybind = a=goto_tab:-1", 1),
            ("keybind = a=write_screen_file:copy,pdf", 1),
        ],
    ),
]


@pytest.fixture(scope="module")
def config(shared_app):
    """The instance's own ghostty.conf and its text at launch, put back once the module is done."""
    path = shared_app.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    yield path, original
    path.write_text(original, encoding="utf-8")


@pytest.mark.covers("config.reload", "field:result.count")
@pytest.mark.parametrize(("lines",), [pytest.param(lines, id=name) for name, lines in CASES])
def test_reload_counts_each_line_the_config_loader_refuses(ctl, config, lines):
    path, original = config
    try:
        for line, diagnostics in lines:
            path.write_text(original.rstrip("\n") + "\n" + line + "\n", encoding="utf-8")
            count = assert_ok(ctl.request("config.reload"))["count"]
            assert count == diagnostics, f"{line!r}: {count} diagnostics, libghostty gives {diagnostics}"
    finally:
        path.write_text(original, encoding="utf-8")
        assert assert_ok(ctl.request("config.reload")) == {"count": 0}


@pytest.mark.covers("config.reload", "field:result.count")
def test_reload_counts_a_theme_refused_as_it_loads(launch):
    """`theme = sub/dir` parses, and Ghostty refuses it as it loads the theme (`theme.zig` `open`: a theme name has no
    path separators): one diagnostic, an unloadable theme reported as any other refused line is. The instance's
    settings name no theme, so the theme loaded is this file's: upstream seeds its own `agterm` theme only on a fresh
    install, with no settings file, and its settings conf sets that over this file; the Rust build seeds none."""
    ctl = launch(settings={"restoreMode": "none"})
    path = ctl.instance.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.rstrip("\n") + "\ntheme = sub/dir\n", encoding="utf-8")
    count = assert_ok(ctl.request("config.reload"))["count"]
    assert count == 1, f"'theme = sub/dir': {count} diagnostics, libghostty gives 1"
