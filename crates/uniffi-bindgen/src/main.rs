//! `uniffi-bindgen-swift`, pinned to the UniFFI version qf-ffi is built with.
//! `scripts/build-mac-core.sh` runs it against the built static library.

fn main() {
    uniffi::uniffi_bindgen_swift()
}
