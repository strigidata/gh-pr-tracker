# prtrack menubar spike

A minimal SwiftUI `MenuBarExtra` over the Rust board, via UniFFI. Command Line Tools only; no Xcode.

```bash
scripts/build-macos-spike.sh                 # builds the static lib and generates bindings
swift build --package-path apps/macos -c release
PRTRACK_REPO=owner/name GH_TOKEN=... apps/macos/.build/release/PrtrackMenuBar
```

The token comes from `GH_TOKEN` or `GITHUB_TOKEN`; the repository from `PRTRACK_REPO`.

`Sources/PrtrackFFI` and `Sources/prtrack_ffiFFI` hold generated code and are
absent from a fresh clone, so `swift build` fails until the script has run. Rerun
the script after any change to `crates/prtrack-ffi`: bindings that no longer
match the library stop the app at its first fetch. No CI job builds this package.
