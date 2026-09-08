# Kairo

Kairo runs reliable WebAssembly Component workflows. It streams data directly
when possible and saves only the checkpoints needed for recovery.

## Quick start

Install Kairo, then work from your project directory:

```bash
cargo install --path crates/cli --locked --root ~/.local --force
kairo init --local
kairo run demos/checkout/workflow.yaml
```

The checkout workflow returns `3207`. Kairo creates durable state internally;
normal runs never need an ID, a database path, or a recovery flag.

```bash
kairo runs
kairo inspect
kairo tui
```

`runs` shows history, `inspect` opens the most recent run, and `tui` provides
a live keyboard-first view. Press `?` inside the TUI for shortcuts and `q` to
exit.

## Local workers

Start a local service in one terminal:

```bash
kairo start --workers 2
```

In another terminal, run workflows normally and watch their queue, worker,
and component progress in the TUI:

```bash
kairo run demos/checkout/workflow.yaml
kairo tui
```

`kairo workers` shows worker health. `kairo start` runs until Ctrl-C and stops
its local workers when it exits.

## Recovery and storage

`durability: required` saves the preceding output as a checkpoint. After an
interruption, rerunning an explicitly named run resumes from its checkpoint:

```bash
kairo run demos/checkout/workflow.yaml --run checkout-retry
```

Use `kairo inspect --verify` to check recorded checkpoints against the active
artifact store. Local storage is the default quick-start backend; `kairo init`
also supports R2 configuration.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```
