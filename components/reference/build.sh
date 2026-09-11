#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
(cd "$root" && cargo build --locked --release --target wasm32-unknown-unknown)
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_doc.wasm" \
    -o "$root/doc/component.wasm"
wasm-tools validate --features cm-async "$root/doc/component.wasm"
