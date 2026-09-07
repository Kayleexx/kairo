# Kairo

Kairo is a locality-aware durable execution runtime for WebAssembly Components:
local when possible, durable when necessary.

The project targets WASI 0.3 and its native Component Model concurrency. The
current runtime provides tracing, resource limits, typed Component execution,
and local linear workflows. Host capabilities are denied unless granted.

## Requirements

- Rust 1.95 or newer

## Try it

```bash
cargo install --path crates/cli --locked --root ~/.local
kairo --help
kairo check components/probe/component.wat
kairo run components/probe/component.wat --input 21
kairo run demos/basic/workflow.yaml
kairo run demos/basic/workflow.yaml --state
kairo run demos/stream/workflow.yaml
kairo run demos/stream/workflow.yaml --materialize
```

The first command installs `kairo` into `~/.local/bin`. The Component check
parses, validates, and compiles the bundled asynchronous probe Component. The
run command invokes its typed WIT export and prints the returned value. Guest
filesystem, network, environment, and host imports are denied by default.
The bundled workflow converts 20°C to 68°F through three real Components.
Workflow component paths are relative to the YAML file. From a directory
containing `workflow.yaml`, simply run `kairo run`.

Pass `--state` to give a scalar workflow a local journal. Kairo derives the
state file from the workflow name and stores it in `.kairo/` in the current
directory. If Kairo is stopped during execution, run the same command again to
reconstruct completed steps and rerun only the unfinished step. Pass
`--state FILE` to use an explicit path instead. Local state protects against
process restarts on the same machine; it is not durable across machine loss.

The stream demo sends a file through two local Components using a bounded
`stream<u8>` connection. It prints the byte count and guest-computed checksum.
Use `--input-file ./data.bin` to supply another file. `--materialize` runs the
same data through the intentional in-memory comparison path. Add `-v` to see
the measured duration, transferred bytes, batch size, and materialized bytes.

## Runtime boundaries

Run one real computation, then verify that resource and capability boundaries
reject unsafe guests:

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
