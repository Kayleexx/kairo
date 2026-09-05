# Kairo

Kairo is a locality-aware durable execution runtime for WebAssembly Components:
local when possible, durable when necessary.

The project targets WASI 0.3 and its native Component Model concurrency. The
current runtime provides tracing, resource limits, typed Component execution,
and explicit host capability configuration. Workflows are not available yet.

## Requirements

- Rust 1.95 or newer

## Try it

```bash
cargo install --path crates/cli --locked --root ~/.local
kairo --help
kairo check components/probe/component.wat
kairo run components/probe/component.wat --input 21
```

The first command installs `kairo` into `~/.local/bin`. The Component check
parses, validates, and compiles the bundled asynchronous probe Component. The
run command invokes its typed WIT export and prints the returned value. Guest
filesystem, network, environment, and host imports are denied by default.

Run a real computation, then verify the runtime boundaries:

```bash
kairo run -v components/runtime/compute.wat --input 5000
kairo run components/runtime/runaway.wat
kairo run components/runtime/memory-limit.wat
kairo run components/runtime/console.wat --input 21
kairo run components/runtime/console.wat --input 21 --allow-console
```

## Development

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```
