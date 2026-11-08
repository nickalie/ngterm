//! Compile the runtime key and the source probe's minimum API version into the app.
fn main() {
    for name in ["KEY", "URL", "SIZE", "SHA256"] {
        println!("cargo::rerun-if-env-changed=NGTERM_WEB_ENGINE_{name}");
    }
    println!("cargo::rerun-if-changed=../../build/pins.env");
    let pins = include_str!("../../build/pins.env");
    let Some(api) = pins.lines().find_map(|line| {
        line.strip_prefix("WPE_API_VERSION=\"")
            .and_then(|value| value.strip_suffix('"'))
    }) else {
        println!("cargo::error=missing WPE_API_VERSION in build/pins.env");
        return;
    };
    println!("cargo::rustc-env=NGTERM_WEB_ENGINE_MIN_VERSION={api}.0");
}
