# Kairo

**Local when possible. Durable when necessary.**

Kairo runs WebAssembly Component workflows. Steps that can run locally stream data directly
between components, as fast as an in-process pipeline. Steps that need to survive a crash or a
restart get a durable checkpoint instead — and only those steps pay for it. You describe a
workflow as a graph of Components; Kairo decides where each step's data goes and recovers your
work if a worker dies partway through.

If you're new to the project, `kairo.md` is the design document and the best place to understand
*why* Kairo is built this way; `AGENTS.md` has the conventions this codebase follows and is the
place to start before opening a pull request.

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

Read `AGENTS.md` first — it covers project structure, coding style, safety rules (no `unsafe`, no
panics in production code), and testing conventions this repository enforces in CI. In short:

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

All four must pass before a pull request is reviewable; CI runs the same checks, plus a file-size
limit (400 lines per hand-written source/test file) and a check that only `README.md` is tracked
as Markdown in this repository.
