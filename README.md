# Kairo

**Build a workflow from reusable Components. Run it locally first. Keep the work worth keeping.**

Kairo runs workflows made from small WebAssembly Components. Add Components to a project, choose
them in a guided workflow builder, then run, inspect, and replay the result from the command line.
Kairo keeps the operational machinery behind the normal flow: local execution is the default;
durable boundaries, workers, and live streaming are selected only when the workflow needs them.

## Requirements

- Rust 1.95+ (`rustup show` to check what you have)
- `wasm-tools`, only if you're building your own Components: `cargo install wasm-tools --locked`
- Docker, only for a local MinIO artifact store (`kairo init --minio`) — plain local files work
  without it

## Install

```bash
git clone https://github.com/Kayleexx/kairo.git
cd kairo
cargo install --path crates/cli --locked --root "$HOME/.local" --force
```

Make sure `$HOME/.local/bin` is on your `PATH`.

## Your first workflow

`kairo init` bundles a few small, real Components with every project, so there's something to try
immediately — no repository checkout needed:

```bash
mkdir my-project && cd my-project
kairo init
kairo new numbers
kairo check numbers
kairo run numbers 5000
```

`kairo new <name>` lists registered Components, shows their real input/output contracts, and
suggests valid connections. It writes ordinary workflow YAML, so the result is portable and easy to
review; there is no separate low-code format. `kairo check` gives a readiness summary before a run.
`kairo run <name> <input>` accepts a file path for a stream workflow or a literal value for a value
workflow, inferred from the workflow itself.

After a run, these are the two commands to remember:

```bash
kairo inspect             # what ran, where it ran, and what it produced
kairo explain             # why Kairo chose its placement and durability strategy
```

The normal flow does not require worker IDs, artifact hashes, or transport details. Use
`--verbose` when those implementation details help diagnose a run.

## Add Components

```bash
kairo add ./decode.wasm
kairo add ./analyze.wasm
kairo new video-analysis
kairo check video-analysis
kairo run video-analysis ./clip.mp4
```

`kairo add` registers a local `.wasm` Component or an OCI reference
(`kairo add ghcr.io/acme/component:v1`) — a tag is resolved once to an immutable digest, verified,
and cached in the project, so a run never depends on a mutable tag. `kairo check <name>` validates
a workflow without running it — useful for CI, not required day to day since `kairo run` validates
the same way before executing.

Building your own Component instead? `kairo component new <name>` scaffolds one, implement its
`src/lib.rs`, `kairo component build` it, then `kairo add` the result.

Use `kairo components` to see what is available, or `kairo component show <name>` to see a
Component's source, version, contract, and digest. A short catalog description helps the guided
builder stay understandable:

```bash
kairo add ./decode.wasm --name decode --description "Turn a video file into frames"
```

## Recipes

A recipe is a named, reusable workflow shape. `kairo recipes` shows each one's real Component
graph before you commit to it:

```bash
kairo recipes
kairo new digest --recipe prime-digest
kairo run digest 7
```

`kairo init` installs two example recipes (`range-prime-count`, `prime-digest`) built from the
bundled starter Components, so they run end to end with nothing else to add. If a recipe
names a Component you haven't registered yet, Kairo prints the exact `kairo add` command to fix
it.

## Understanding a run

`kairo inspect <run>` reads back the recorded result, inputs, outputs, and recovery history.
`kairo explain <run>` reads back the measured placement, transport, durability, and cost decisions.
Every reported number is measured on that run or clearly marked as an earlier estimate; Kairo does
not invent a second explanation after the fact.

## Named runs, waits, and cleanup

```bash
kairo run <workflow> --run approval-flow
kairo signal approval-flow
kairo cancel approval-flow
kairo resume approval-flow
```

A wait doesn't tie up a worker, and retrying after a crash never repeats an external action.
`kairo resume` picks a run back up from its last checkpoint.

```bash
kairo prune                              # see what would be removed
kairo prune --yes --older-than-hours 24  # only finished runs are touched
```

## Replaying a stream run

```bash
kairo run workflow.yaml ./input.bin --watch --workers 2 --run import-run
kairo replay import-run --until analyze
```

Kairo reuses the nearest durable boundary before the target step when one exists, otherwise
reruns from the verified original input. The source run is never modified. Replay refuses
workflows with waits or external effects, since those can't safely happen twice.

## Keep a local runtime running

```bash
kairo up --scale 2
kairo run numbers 5000
kairo status
kairo down
```

This is optional for a first local run. `kairo up`/`kairo down` (aliases: `start`/`stop`) keep a
small local runtime running between separate invocations, which is useful for managed multi-worker
work and recovery after a worker dies.

## Measuring performance

```bash
kairo bench run checkout-settlement --warmups 1 --repetitions 10
kairo bench run checkout-settlement --failure-scenario worker-kill --repetitions 3
```

`kairo run` already measures and caches a durability plan the first time it needs one; `kairo
bench` is for a fuller, on-purpose measurement.

## Checking your setup

```bash
kairo doctor
kairo storage check --input ./some-file
```

`kairo doctor` reports what's missing and how to fix it (`--fix` repairs what it can). Inside a
Kairo checkout, it also flags when your installed binary doesn't match the current source.

To use Cloudflare R2 instead of local files, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`,
`KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` (environment or a local `.env`, never
committed), then run `kairo init --r2`.

## Bundled examples

These only work from inside this checkout, since that's where their files live:

```bash
kairo run checkout-settlement
kairo workflows
```

`kairo workflows` lists everything available and what each one expects as input.

## Commands

| Command | What it does |
|---|---|
| `kairo init` | Set up Kairo in the current folder |
| `kairo add <path-or-oci-ref>` | Validate, pin, and register a Component |
| `kairo components` | List registered Components and their contracts |
| `kairo component show <name>` | Show a Component's contract, source, and digest (alias: `info`) |
| `kairo new [name]` | Build a workflow from the catalog interactively, or via `--recipe` |
| `kairo recipes` | Preview reusable workflow recipes |
| `kairo check <name>` | Validate a workflow or Component without running it |
| `kairo run <name> [input]` | Run a workflow or Component |
| `kairo inspect [run]` | Show what happened; defaults to the most recent run |
| `kairo explain [run]` | Show why: placement, transport, durability, and cost |
| `kairo resume <run>` | Continue a stopped run from its last checkpoint |
| `kairo replay <run> --until <step>` | Create a child run from a completed stream run's boundary |
| `kairo runs` | List local runs |
| `kairo signal <run> [name]` | Send a signal to a run waiting for one |
| `kairo cancel <run>` | Cancel a queued, waiting, or running run |
| `kairo prune` | Remove finished run records |
| `kairo up` / `kairo down` | Start or stop a persistent local runtime |
| `kairo status` / `kairo workers` | Check the runtime and its workers |
| `kairo doctor` | Diagnose and optionally fix local setup problems |
| `kairo tui` | Terminal dashboard for composing and watching workflows |

Add `--json` for machine-readable output, `--quiet` for just the result, `--verbose` for full
diagnostics and hashes.

## Contributing

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

No `unsafe` code, no panics in normal paths, real tests over mocked ones.
