# Kairo

Kairo runs reliable WebAssembly Component workflows. It streams locally when
possible and records only the recovery boundaries that matter.

## Quick start

Install the CLI, then run your workflow from its project directory:

```bash
cargo install --path crates/cli --locked --root ~/.local --force
kairo run workflow.yaml --watch
```

Kairo prepares local durable storage when needed, starts temporary workers,
opens live activity, and cleans them up when you exit. You do not need to name
runs, manage state files, or start an effect service for this path.

Inside live activity, use `↑↓` to choose a run, `Enter` for details, `s` then
`Enter` to release a selected signal wait, `r` to refresh, `?` for help, and
`q` to quit.

## Persistent local service

Use this when you want workers to remain available across several runs:

```bash
kairo start --workers 4
kairo tui
```

`start` returns once the project-local service is ready. Run workflows from the
same terminal after leaving the TUI, or use another shell if preferred:

```bash
kairo run workflow.yaml
kairo workers
kairo runs
kairo inspect
kairo stop
```

Use `kairo start --foreground` only when troubleshooting the service itself.

## Waiting and external actions

Timers and signals are saved safely without occupying a worker. The TUI shows
what a run is waiting for and can send its selected signal. Automation can use:

```bash
kairo signal RUN_NAME SIGNAL_NAME
```

Workflows that declare an external action automatically receive the local
idempotent provider. Kairo stores a receipt before the action and reuses its
stable identity after worker recovery, so the local provider does not create a
duplicate logical action. `kairo inspect` shows its recorded status.

## Create a workflow

Use the guided creator when you do not want to write YAML:

```bash
kairo workflow create
```

It asks for a workflow name, one or more Components, step names, durability,
optional waits/effects, and whether to run immediately. The generated file is
validated against the real Component interfaces before it is written.

For scripts, use the short non-interactive form:

```bash
kairo workflow create --name multiply --component path/to/component.wasm --input 21
kairo run multiply
```

The creator lists Components found in `components/`, accepts a listed number
or path, previews the graph, and validates every Component before saving. Add
`--durability required`, `--wait timer:1000` (or `signal:name`), `--effect
operation`, and `--run` when scripting the same flow. It creates scalar linear
workflows; branching or stream workflows remain fully supported through YAML.

The existing `kairo new workflow ...` command remains available for its
single-Component shorthand.

## Recovery and verification

`durability: required` saves the preceding output as a checkpoint. To give a
run a stable recovery name for automation:

```bash
kairo run workflow.yaml --run nightly-import
kairo inspect nightly-import --verify
```

For local failure testing, use `kairo chaos kill WORKER_NAME`; another healthy
worker resumes durable work with stale-worker protection.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```
