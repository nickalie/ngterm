//! `UniFFI`'s bindgen, run in library mode over a build of the library to write the mobile apps' bindings, as
//! `uniffi.toml` configures them: the Kotlin ones for Android over the built `libngterm_mobile.so`
//! (scripts/build-android.sh), and the Swift ones for iOS over the host build of the library (the interface is read
//! from the library's metadata, the same on every target):
//! `uniffi-bindgen generate --library <library> --language kotlin|swift --no-format --out-dir <dir>`.

fn main() {
    uniffi::uniffi_bindgen_main();
}
