# Kairo

**Local when possible. Durable when necessary.**

Kairo runs workflows made out of small building blocks called Components. A Component is a
self-contained piece of code (a WebAssembly Component) that does one job, like transforming some
bytes or checking a value. A workflow chains a few Components together, and Kairo runs each step
for you.

Most steps just run fast, one after another. But sometimes a step's result is too important to
lose. If a machine crashes halfway through a run, you want to pick up where you left off instead
of starting over. You tell Kairo which steps need that safety net, and it keeps everything else
fast and simple. You never have to know how Kairo runs things internally. You create a workflow,
give it a name, run it, and check on it later.

## What you need

- Rust 1.95 or newer. Run `rustup show` to check what you have.
- `wasm-tools`, needed only when building your own Component. Install it with
  `cargo install wasm-tools --locked` and make sure `~/.cargo/bin` is on your `PATH`.
- Docker, only if you want a local MinIO artifact store via `kairo init --minio`. Plain local
  files work fine without it.

## Install

```bash
git clone https://github.com/Kayleexx/kairo.git
cd kairo
cargo install --path crates/cli --locked --root "$HOME/.local" --force
```

Make sure `$HOME/.local/bin` is on your `PATH`, then `kairo` is ready to use anywhere.

## Build your first workflow

```bash
mkdir my-project && cd my-project
kairo init
kairo new
```

`kairo init` sets up a small local project folder. `kairo new` asks a few plain questions: a
name for the workflow, then for each step it shows the real, reusable Components that can
legally come next, with what each one does and its input/output shape, so you compose real
behavior instead of guessing. Type a name that doesn't match anything, and Kairo says so
plainly instead of quietly inventing an empty stub. You choose to search again, import a
Component, or create a new one yourself. No code up front, no file paths, no hand-edited YAML.

Now run it:

```bash
kairo run greet --value hello
kairo inspect
```

`kairo run` runs the workflow with the value you gave it. The first time a workflow needs to
decide whether it's cheaper to redo a step from scratch or save its result after a crash, Kairo
measures that once, quietly, and remembers the answer. `kairo inspect` shows what happened:
which steps ran, how long each took, and what got saved along the way.

Ready to write real logic? Open the new component's `src/lib.rs` under `components/warm-up/`
(plain Rust) and run `kairo component build components/warm-up` to rebuild it.

Prefer one line over answering prompts? `kairo workflow new greet warm-up finish` does the same
thing non-interactively. Use this form in scripts.

## Try the bundled examples

The repository ships with ready-made workflows you can run without building anything. These
only work from inside a checkout of this repository, since that's where their files live:

```bash
cd kairo
kairo run checkout-settlement
kairo inspect
```

Run `kairo workflows` any time to see the full list, what each one expects as input, and what
it hands back:

- **video**: reports frame count, size, brightness, and frame-to-frame change in an MP4 or Y4M
  clip.
- **doc**: counts lines, words, characters, and paragraphs in a text or DOCX file.
- **invoice**: checks and totals invoice records from a JSONL or CSV file.
- **redact**: finds and blanks out one email-looking string or 10-digit number in some text.
- **preview**: turns a short video clip into a grayscale contact-sheet image.
- **approval**: pauses and waits for an approval signal before continuing.
- **delay**: waits for a set amount of time without tying up a worker.
- **order**: calls an outside "create order" action safely, so running it twice never books it
  twice.

## Commands you'll use most

| Command | What it does |
|---|---|
| `kairo init` | Set up Kairo in the current folder |
| `kairo new` | Create a workflow by answering a few plain questions |
| `kairo workflow new <name> [steps...]` | Same thing, non-interactive, for scripts |
| `kairo run <name> [--value X]` | Run a workflow (measures and plans automatically) |
| `kairo run <name> --value X --watch` | Run it and watch progress as it happens |
| `kairo run <name> --run <run-name>` | Run it under a name you can find again later |
| `kairo inspect [<run>]` | See what happened; defaults to the most recent run |
| `kairo explain [<run>]` | See why: placement, transport, durability, real cost per step |
| `kairo resume <run>` | Pick a stopped run back up from its last safe point |
| `kairo replay <run> --until <step>` | Create a child run from a completed stream run's safe boundary |
| `kairo up --scale N` | Start a small local runtime with N workers, in the background |
| `kairo status` | Check whether that runtime is up and ready |
| `kairo down` | Stop the runtime |
| `kairo runs` | List the runs you've done locally |
| `kairo signal <run> [name]` | Send a signal to a run that's waiting for one |
| `kairo cancel <run>` | Cancel a run that's queued, waiting, or in progress |
| `kairo prune` | Clean up old, finished run records |
| `kairo doctor` | Check that your local setup is healthy, and explain what's wrong |
| `kairo tui` | Open a dashboard for composing, running, and watching workflows |

Add `--json` for machine-readable output, or `--quiet` to just get the result. Add `--verbose`
for runtime diagnostics and full hashes; you won't need it for normal use.

## Explaining a decision

`kairo inspect` tells you what happened. `kairo explain` tells you why: which worker ran each
step, whether it stayed local or went through durable storage, whether that step's durability
was your decision or the planner's, and the real cost behind it.

```bash
kairo explain checkout-settlement
kairo explain checkout-settlement --json
```

Every number shown was either measured on a real run or clearly marked as an estimate from an
earlier one. Nothing is invented. Something Kairo genuinely doesn't know shows up as `unknown`,
not `0`.

## Named runs, waits, and cleanup

Give a run a name whenever you'll want to find it again, to signal it, check on it, cancel it,
or resume it after a crash:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo cancel approval-flow
kairo resume approval-flow
```

A wait like this doesn't tie up a worker, and a retry after a crash never triggers an outside
action twice. `kairo resume` picks a run back up from its last saved checkpoint, so nothing
already saved gets redone.

Named runs leave a small record on disk that nothing removes automatically:

```bash
kairo prune                 # see what would be removed
kairo prune --yes --older-than-hours 24
```

`kairo prune` only touches runs that have already finished. Anything still in progress is left
alone.

## Replaying a completed stream run

To rerun just a safe suffix of a completed stream workflow, create a child run:

```bash
kairo replay video-run --until analyze-video
kairo inspect
kairo explain
```

The source run never changes. Kairo verifies the original workflow, Components, and input before
it starts; when a matching durable boundary is available, it reuses that boundary and reruns only
what follows it. Otherwise it starts again from the verified original input. Replays refuse
workflows with waits or external effects unless Kairo can prove a receipt is safe to reuse.

## Running as a background service

```bash
kairo up --scale 2
kairo run checkout-settlement
kairo status
kairo down
```

`kairo up` starts a small local runtime that keeps running between separate `kairo run`
commands, so workflows are properly scheduled and can recover automatically if something goes
wrong.

For stream workflows, Kairo keeps adjacent Components in one process when possible. When work
is placed on different workers, an ephemeral edge can stream incrementally over bounded QUIC
without writing the whole edge to artifact storage. A required edge still uses a durable
artifact. `kairo inspect` and `kairo explain` report the transport that actually ran, including
the worker pair and byte counts, rather than repeating the planner's intention.

If a worker or the control service disappears during a live stream, the partial stream is
discarded and the run recovers from its latest valid durable boundary. Partial-stream replay is
not supported.

## Measuring performance

`kairo bench` is something you reach for on purpose; it never runs as part of a normal `kairo
run`. It runs a workflow repeatedly for real and writes a report, every number actually
measured:

```bash
kairo bench run checkout-settlement --warmups 1 --repetitions 10
kairo bench run checkout-settlement --failure-scenario worker-kill --repetitions 3
kairo bench list
```

`kairo run` already measures and caches a plan for you the first time it needs one. For a
fuller measurement, run `kairo workflow profile <name>` yourself at any time.

## Checking your setup

```bash
kairo doctor
kairo storage check --input ./some-file
```

`kairo doctor` tells you what's missing or set up wrong and how to fix it. `kairo storage
check` confirms your storage backend can be written to and read from; add `--input <path>` to
send a real file through it.

To use Cloudflare R2 instead of local files, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`,
`KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` (environment or a local `.env`, never
committed), then run `kairo init --r2`.

## Contributing

Before opening a pull request, make sure these all pass. CI runs the same checks:

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The codebase avoids unsafe code and panics in normal paths, keeps files reasonably sized, and
prefers real tests over mocked ones.
