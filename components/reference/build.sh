#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
(cd "$root" && cargo build --locked --release --target wasm32-unknown-unknown)
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_doc.wasm" \
    -o "$root/doc/component.wasm"
wasm-tools validate --features cm-async "$root/doc/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_doc_redact.wasm" \
    -o "$root/doc-redact/component.wasm"
wasm-tools validate --features cm-async "$root/doc-redact/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_video.wasm" \
    -o "$root/video/component.wasm"
wasm-tools validate --features cm-async "$root/video/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_video_frame.wasm" \
    -o "$root/video-frame/component.wasm"
wasm-tools validate --features cm-async "$root/video-frame/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_expand_range.wasm" \
    -o "$root/expand-range/component.wasm"
wasm-tools validate --features cm-async "$root/expand-range/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_count_primes.wasm" \
    -o "$root/count-primes/component.wasm"
wasm-tools validate --features cm-async "$root/count-primes/component.wasm"
wasm-tools component new \
    "$root/target/wasm32-unknown-unknown/release/kairo_digit_sum.wasm" \
    -o "$root/digit-sum/component.wasm"
wasm-tools validate --features cm-async "$root/digit-sum/component.wasm"
