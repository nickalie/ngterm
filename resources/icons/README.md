# Icons

The ngterm mark is a copper gear with a `>_` prompt. `ngterm-logo.png` is the full logo with the wordmark. Every
icon here comes from `ngterm.svg`, the gear without the wordmark, traced from that logo.

| File | Used for |
|---|---|
| `ngterm.svg` | Master artwork; renders crisply at any size |
| `hicolor/<size>/apps/com.nickalie.ngterm.png`, `hicolor/scalable/apps/com.nickalie.ngterm.svg` | Linux launcher, dock and window icon. Install the tree under `/usr/share/icons/hicolor` (or `~/.local/share/icons/hicolor`) and point the `.desktop` file at `Icon=com.nickalie.ngterm`. The compositor finds the window icon through the app id. Sizes: 16, 22, 24, 32, 48, 64, 128, 256, 512 |
| `ngterm.ico` | Windows executable, taskbar and window icon. Sizes: 16, 20, 24, 32, 40, 48, 64, 256 (PNG-compressed) |
| `ngterm.icns` | macOS bundle icon (`CFBundleIconFile`). 16–1024 px plus @2x variants, with the artwork inset to Apple's 824/1024 grid |
| `ngterm-logo.png` | Full logo with the wordmark, for the README and website |

To regenerate a PNG size, render it from the SVG, for example
`rsvg-convert -w 256 -h 256 ngterm.svg -o hicolor/256x256/apps/com.nickalie.ngterm.png`.
