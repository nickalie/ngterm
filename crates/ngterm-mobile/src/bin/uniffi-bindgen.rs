//! `UniFFI`'s bindgen, run in library mode over the built `libngterm_mobile.so` to write the Kotlin bindings
//! (scripts/build-android.sh): `uniffi-bindgen generate --library <.so> --language kotlin --out-dir <dir>`.

fn main() {
    uniffi::uniffi_bindgen_main();
}
