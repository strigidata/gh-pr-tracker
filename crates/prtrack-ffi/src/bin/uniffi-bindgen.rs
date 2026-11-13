//! The binding generator, pinned to this crate's uniffi version.
//!
//! Built only with `--features bindgen`. Running the generator from here
//! rather than from a separately installed `uniffi-bindgen` keeps the bindings
//! and the scaffolding they talk to on the same uniffi release, which the
//! generated checksums insist on anyway.

#![forbid(unsafe_code)]

fn main() {
    uniffi::uniffi_bindgen_main();
}
