# Kairo

Kairo is a locality-aware durable execution runtime for WebAssembly Components:
local when possible, durable when necessary.

The project targets WASI 0.3 and its native Component Model concurrency. The
current runtime provides the workspace, tracing, shared primitives, a minimal
CLI, and a real Component loading path. Workflow execution and WASI host
capabilities are not available yet.

## Requirements

- Rust 1.95 or newer

## Try it

```bash
cargo install --path crates/cli --locked
kairo --help
kairo component check components/probe/component.wat
```

The first command installs `kairo` into Cargo's binary directory. The Component
check parses, validates, and compiles the bundled asynchronous probe Component
without executing it.

The current runtime does not run Component functions or workflows yet.

## Development

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```
