# Kairo

**Local when possible. Durable when necessary.**

Kairo runs WebAssembly Component workflows. Steps that can run locally stream data directly
between components, as fast as an in-process pipeline. Steps that need to survive a crash or a
restart get a durable checkpoint instead — and only those steps pay for it. You describe a
workflow as a graph of Components; Kairo decides where each step's data goes and recovers your
work if a worker dies partway through.

## Requirements

- Rust `1.95` or newer (`rustup show` to check; `rust-toolchain`-compatible toolchains work too).
- Docker, only if you want a local MinIO artifact store via `kairo init --minio`. Everything else
  works without Docker.

## Quick start

```bash
cargo install --path crates/cli --locked --root "$HOME/.local" --force
kairo init
kairo run checkout-settlement
kairo inspect
```

`kairo init` sets up local artifact storage and a `.kairo/` project directory. `kairo run
checkout-settlement` runs a bundled demo workflow end to end. `kairo inspect` shows what just
happened — which components ran, how long they took, and what got checkpointed.

## Core commands

| Command | Description |
|---|---|
| `kairo run <workflow>` | Run a workflow (scalar or stream) |
| `kairo run <workflow> --watch` | Live-watch a workflow run |
| `kairo run <workflow> --run <name>` | Run it as a named, durable, resumable run |
| `kairo up --workers N` | Start a persistent service with N workers |
| `kairo down` | Stop the service |
| `kairo tui` | Multi-run terminal dashboard |
| `kairo runs` | List local runs (alias: `kairo cells`) |
| `kairo inspect [<run>]` | Post-run detail view; defaults to the most recent run |
| `kairo inspect <run> --export <path>` | Re-export a run's output artifact without rerunning it |
| `kairo inspect <run> --verify` | Verify a run's checkpoints against artifact storage |
| `kairo workflows [<path>]` | List observed workflows, or show one workflow's graph |
| `kairo signal <run> [<name>]` | Send a signal to a run that's waiting for one |
| `kairo cancel <run>` | Cancel a queued, waiting, or running run |
| `kairo prune [--older-than-hours N] [--workflow NAME]` | Preview or (`--yes`) delete finished local run journals |
| `kairo chaos kill <worker>` | Kill a worker for recovery testing |
| `kairo bench run <workflow>` | Measure real repeated executions; writes a JSON report |
| `kairo doctor` | Check local setup and explain problems |
| `kairo storage check [--input <path>]` | Verify the artifact store, optionally round-tripping a file through it |

Every command also accepts `--json` (machine-readable output, where supported — currently `runs`
and `doctor`) and `--quiet` (suppress progress lines; the primary result still prints), so scripts
and CI can drive Kairo without parsing human-formatted text.

## Reference workflows

Included demos (run with `kairo run <name>`):

- **video** - Analyze H.264/AVC MP4 or Y4M: frame count, dimensions, average luma, frame-to-frame luma change. Max 24 decoded 8-bit 4:2:0 frames; audio ignored; MP4 limited to 6 MiB and 1280x720.
- **doc** - Count lines, words, characters, paragraphs, longest line for UTF-8 text or extracted DOCX text.
- **invoice** - Validate and aggregate structured invoice records in JSONL or CSV.
- **redact** - Redact one ASCII email-like token or one 10-digit token at a time from text; normalizes CRLF to LF.
- **preview** - Grayscale contact-sheet PNG from up to 24 decoded frames of a bounded H.264/AVC MP4.
- **approval** - Wait for an `approval.granted` signal; use `kairo signal <run>` to resume it.
- **delay** - Durable timer wait; the worker is released while waiting, not blocked.
- **order** - Idempotent `create-order` external effect.

`kairo workflows` lists every workflow Kairo has observed locally, reference demos included, along
with their expected input and result shape.

## Persistent runtime

```bash
kairo up --workers 4
kairo run checkout-settlement
kairo tui
kairo down
```

`kairo up` starts a local control plane and worker pool that keeps running across separate `kairo
run` invocations, so scalar workflows can be scheduled, retried, and recovered instead of running
inline in the CLI process.

## Create a workflow

```bash
kairo workflow create
```

A guided prompt asks for a name, the Components to chain together, their order, and the input.
Pass `--advanced` to also configure durability, waits, and external effects.

## Durable workflows

Give a run a name when you'll need to refer to that specific execution later — to send it a
signal, inspect it, or cancel it:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo inspect approval-flow --verify
kairo cancel approval-flow
```

Timers wait without occupying a worker. Effects use durable receipts and idempotency keys, so a
retried effect after a crash never double-runs the external action.

## Locality-aware execution

When a multi-worker deployment (`kairo up`/`kairo start`) runs a scalar workflow with `durability:
required` boundaries, Kairo may split it into contiguous **ExecutionGroups**, each pinned to one
worker. Components inside a group always talk directly, in-process — a group boundary only ever
crosses workers by handing over an already-committed durable artifact, never a live stream. This
is entirely automatic: `kairo run <workflow> [input]` never changes, and a run only ever moves when
there's a real, cheap-to-hand-off boundary and a genuinely idle worker to take it — never just
because a worker happens to be free. `kairo inspect <run>` shows a `placement` section whenever a
run's groups actually moved between workers.

## Local run housekeeping

Every durable run leaves a small SQLite journal under `.kairo/`. Nothing deletes these
automatically, so long-running local development can accumulate a lot of them:

```bash
kairo prune                 # preview what's safe to remove
kairo prune --yes           # actually remove it
kairo prune --older-than-hours 24 --workflow checkout-settlement
```

`kairo prune` only ever removes runs that have reached a terminal state (completed, failed, or
canceled) and aren't currently locked by a running process — a run still in progress is always
left alone.

## Benchmarking

`kairo bench` is an opt-in diagnostic command — it never runs as part of `kairo run`, and normal
use of Kairo never needs it. It drives repeated real executions of a workflow and writes a JSON
report; every field is either a real measurement or absent, never invented.

```bash
kairo bench run checkout-settlement --warmups 1 --repetitions 10
kairo bench run redact sample.txt --repetitions 5     # stream workflows work too
kairo bench list
kairo bench show <report-file>
```

Reports land under `.kairo/benchmarks/<workflow>-<time>.json` by default (or `--output PATH`) and
never silently overwrite an existing file. A failed or canceled attempt is recorded under
`failures`, never counted toward `summary`.

To measure real worker-crash recovery instead of plain timing (scalar workflows only):

```bash
kairo bench run checkout-settlement --failure-scenario worker-kill --repetitions 3
```

Each repetition starts a fresh local service, submits the run through the control plane, waits
for a real worker to pick it up, sends it a real `SIGKILL`, and measures how long recovery on a
surviving worker actually takes — the same mechanism behind `kairo chaos kill`. This takes over
the local service for the duration of the benchmark.

## Diagnose and configure

```bash
kairo doctor
kairo storage check
kairo storage check --input ./some-file
kairo workers
```

`kairo doctor` explains what's missing or misconfigured and suggests the fix. `kairo storage
check` proves the configured artifact store can be written to and read from; add `--input <path>`
to also round-trip a real file through it and confirm the bytes come back identical.

For Cloudflare R2, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`, `KAIRO_R2_ACCESS_KEY_ID`,
and `KAIRO_R2_SECRET_ACCESS_KEY` in the environment or a local `.env`, then run `kairo init --r2`.
Kairo never writes credentials to source, and `.env` is gitignored.

## Contributing

This repository enforces safety rules (no `unsafe`, no panics in production code), formatting,
and testing conventions in CI. Before opening a pull request:

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

All four must pass before a pull request is reviewable; CI runs the same checks, plus a file-size
limit (400 lines per hand-written source/test file) and a check that only `README.md` is tracked
as Markdown in this repository.
