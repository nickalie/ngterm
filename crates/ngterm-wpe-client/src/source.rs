//! Library and module paths applied only to the engine worker's command.

use std::path::PathBuf;
use std::process::Command;

/// The engine installation selected by the app before starting a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineSource {
    /// The system engine (or the development build's vendored installation).
    System,
    /// A complete, verified runtime installation.
    Runtime(PathBuf),
}

/// Worker-only startup flag: a relocatable runtime needs no development overlay.
#[doc(hidden)]
pub const RUNTIME_ENV: &str = "NGTERM_WPE_RUNTIME";

impl EngineSource {
    pub(crate) fn configure(&self, command: &mut Command) {
        let Self::Runtime(root) = self else {
            // Match the system probe: only the installed library search paths may select the engine.
            command.env_remove("LD_LIBRARY_PATH");
            return;
        };
        for (name, relative) in [
            ("LD_LIBRARY_PATH", "lib"),
            ("GIO_MODULE_DIR", "lib/gio/modules"),
            ("GST_PLUGIN_SYSTEM_PATH_1_0", "lib/gstreamer-1.0"),
            ("GST_PLUGIN_SCANNER", "libexec/gstreamer-1.0/gst-plugin-scanner"),
            ("GST_REGISTRY_1_0", "gstreamer-registry.bin"),
            ("WEBKIT_INJECTED_BUNDLE_PATH", "lib/wpe-webkit-2.0/injected-bundle"),
        ] {
            command.env(name, root.join(relative));
        }
        for name in ["GIO_EXTRA_MODULES", "GST_PLUGIN_PATH", "GST_PLUGIN_PATH_1_0"] {
            command.env_remove(name);
        }
        command.env(RUNTIME_ENV, "1");
    }
}
