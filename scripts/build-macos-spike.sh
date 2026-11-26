#!/usr/bin/env bash
# Builds the static library the macOS spike links and generates its Swift
# bindings. Host architecture only: a universal build is out of the spike's
# scope.
#
# Bindgen reads the metadata straight out of the static archive, so no dynamic
# library is built. Everything this writes under apps/macos is generated and
# ignored by git; rerun this after any change to crates/prtrack-ffi.
set -euo pipefail
cd "$(dirname "$0")/.."

readonly library=target/release/libprtrack_ffi.a
readonly generated=apps/macos/Generated
readonly swift_dir=apps/macos/Sources/PrtrackFFI
readonly header_dir=apps/macos/Sources/prtrack_ffiFFI/include

cargo build --release --locked -p prtrack-ffi
cargo run --release --locked -p prtrack-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library "$library" --language swift --out-dir "$generated"

mkdir -p "$swift_dir" "$header_dir"
cp "$generated/prtrack_ffi.swift" "$swift_dir/"
cp "$generated/prtrack_ffiFFI.h" "$header_dir/"
cp "$generated/prtrack_ffiFFI.modulemap" "$header_dir/module.modulemap"
