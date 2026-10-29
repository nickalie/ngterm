//! The app's icon on its windows (`resources/icons`, whose README lists where each form goes). Wayland has no
//! per-window icon: the compositor finds it through the window's app id, the desktop entry of that name and the icon
//! theme's `hicolor` tree. X11 reads the window's own `_NET_WM_ICON`, which GPUI sets from
//! [`gpui_kit::WindowOptions::icon`]; [`window_icon`] is that image.

use std::sync::{Arc, LazyLock};

use image::RgbaImage;

/// The icon's 256 px rendering, the largest a window manager's switcher or taskbar asks for.
const PNG: &[u8] = include_bytes!("../../../resources/icons/hicolor/256x256/apps/com.nickalie.ngterm.png");

/// The decoded icon, once per process.
static ICON: LazyLock<Option<Arc<RgbaImage>>> = LazyLock::new(|| match image::load_from_memory(PNG) {
    Ok(decoded) => Some(Arc::new(decoded.into_rgba8())),
    Err(error) => {
        tracing::warn!(%error, "the app icon did not decode; windows open without one");
        None
    }
});

/// The window icon every window of the app carries; `None` (logged once) if the bundled image does not decode.
#[must_use]
pub fn window_icon() -> Option<Arc<RgbaImage>> {
    ICON.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_icon_decodes_at_its_size() {
        let icon = window_icon().expect("the bundled icon decodes");
        assert_eq!((icon.width(), icon.height()), (256, 256));
    }
}
