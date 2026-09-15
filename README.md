# Kairo

**Local when possible. Durable when necessary.**

Kairo runs workflows made out of small building blocks called Components. A Component is a
self-contained piece of code (a WebAssembly Component) that does one job, like transforming some
bytes or checking a value. A workflow chains a few Components together into a small pipeline, and
Kairo runs each step for you.

Most steps should just run fast, one after another, with no extra overhead. But sometimes a
step's result is too important to lose. If a machine crashes halfway through a run, you want to
pick up exactly where you left off instead of starting over from the beginning. You tell Kairo
which steps need that safety net, and it leaves everything else fast and simple. You describe the
workflow, and Kairo figures out where each step's data should live and brings your run back if
something dies partway through.

You do not need to know anything about how Kairo runs things internally to use it. You just
create a workflow, give it a name, run it, and check on it later.

## What you need

- Rust 1.95 or newer. Run `rustup show` to check what you have installed.
- `wasm-tools`, used to package a component (`cargo install wasm-tools` if you do not have it).
- Docker, but only if you want a local MinIO artifact store via `kairo init --minio`. Everything
  else works fine without it, using plain local files by default.

## Install

```bash
git clone https://github.com/Kayleexx/kairo.git
cd kairo
cargo install --path crates/cli --locked --root "$HOME/.local" --force
```

Make sure `$HOME/.local/bin` is on your `PATH`, then the `kairo` command is ready to use anywhere.

## Build your first workflow

This is the fastest way to see the whole idea. It works from any empty folder.

```bash
mkdir my-project && cd my-project
kairo init
kairo workflow new greet warm-up finish
```

`kairo init` sets up a small local project folder for Kairo to keep its state in. `kairo workflow
new greet warm-up finish` creates a workflow called `greet` made of two steps, `warm-up` and
`finish`. Since neither of those steps exists yet as a Component, Kairo builds simple starter
Components for you automatically, wires them together, and writes `greet.yaml`. You never had to
write any code, pick a file path, or edit any YAML by hand.

Now run it and give it a value to work with:

```bash
kairo workflow profile greet --value hello
kairo run greet --value hello
kairo inspect
```

`kairo workflow profile` measures the workflow once so Kairo can decide, per step, whether it is
cheaper to redo a step from scratch or save its result if a crash happens. `kairo run` actually
runs the workflow with the value you gave it. `kairo inspect` shows you what just happened, which
steps ran, how long each one took, and what got saved along the way.

When you are ready to write real logic instead of the starter code, open the new component's
`src/lib.rs` file under `components/warm-up/` (plain Rust) and run `kairo component build
components/warm-up` to rebuild it.

## Try the bundled examples

The repository also ships with a handful of ready-made workflows you can run without building
anything yourself. These only work from inside a checkout of this repository, since that is where
their files live:

```bash
cd kairo
kairo run checkout-settlement
kairo inspect
```

Run `kairo workflows` any time to see the full list of bundled examples, what each one expects as
input, and what it hands back:

- **video**, looks at an MP4 or Y4M clip and reports frame count, size, brightness, and how much
  the picture changes from frame to frame.
- **doc**, counts lines, words, characters, and paragraphs in a text file or a DOCX file.
- **invoice**, checks and totals up invoice records from a JSONL or CSV file.
- **redact**, finds and blanks out one email-looking string or one 10-digit number in some text.
- **preview**, turns a short video clip into a grayscale contact-sheet image.
- **approval**, pauses and waits for someone to send an approval signal before it continues.
- **delay**, waits for a set amount of time without tying up any worker while it waits.
- **order**, calls an outside "create order" action safely, so running it twice never books it
  twice.

## Commands you will use most

| Command | What it does |
|---|---|
| `kairo init` | Set up Kairo in the current folder |
| `kairo workflow new <name> [steps...]` | Create a workflow, building any missing steps for you |
| `kairo workflow profile <name> [--value X]` | Measure a workflow once so Kairo can plan around it |
| `kairo run <name> [--value X]` | Run a workflow |
| `kairo run <name> --value X --watch` | Run it and watch progress as it happens |
| `kairo run <name> --run <run-name>` | Run it under a name you can find and come back to later |
| `kairo inspect [<run>]` | See what happened during a run, defaults to the most recent one |
| `kairo resume <run>` | Pick a stopped run back up from its last safe point |
| `kairo up --scale N` | Start a small local runtime with N workers, running in the background |
| `kairo status` | Check whether that runtime is up and ready |
| `kairo down` | Stop the runtime |
| `kairo runs` | List the runs you have done locally |
| `kairo signal <run> [name]` | Send a signal to a run that is waiting for one |
| `kairo cancel <run>` | Cancel a run that is queued, waiting, or in progress |
| `kairo prune` | Clean up old, finished run records |
| `kairo doctor` | Check that your local setup is healthy and explain what is wrong if not |
| `kairo tui` | Open a dashboard for watching and running workflows |

Add `--json` for machine-readable output, or `--quiet` to skip the extra progress lines and just
get the result. Both are handy for scripts.

Add `--verbose` to any command to see more detail, such as which worker ran which step. You will
not need it for normal use.

## A workflow you can find again later

Give a run a name whenever you think you will want to find it again, to send it a signal, look at
what happened, or cancel it:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo inspect approval-flow --verify
kairo cancel approval-flow
```

A wait like this does not tie up a worker while it waits. And if a workflow calls out to something
outside Kairo, a retry after a crash will never trigger that outside action twice.

## Picking a run back up

If a run gets interrupted (a crash, a killed process, a machine restart) and it had already saved
a safe checkpoint, you do not need to start over:

```bash
kairo resume checkout-settlement
```

This picks the run up from its last saved checkpoint. Whatever work was already saved is not
redone.

## Running a small service in the background

```bash
kairo up --scale 2
kairo run checkout-settlement
kairo status
kairo down
```

`kairo up` starts a small local runtime that keeps running between separate `kairo run` commands.
With it running, workflows are properly scheduled and can recover automatically if something goes
wrong, instead of just running once inline in your terminal. `kairo status` tells you it is ready
and how much capacity it has, without showing you any internal detail.

## Cleaning up old runs

Every run that was given a name leaves behind a small record on disk. Nothing deletes these for
you automatically, so if you have been developing locally for a while, they can pile up:

```bash
kairo prune                 # see what would be removed
kairo prune --yes           # actually remove it
kairo prune --older-than-hours 24 --workflow checkout-settlement
```

`kairo prune` only ever touches runs that have actually finished (completed, failed, or canceled).
Anything still in progress is always left alone.

## Measuring performance

`kairo bench` is something you reach for on purpose. It never runs as part of a normal `kairo
run`. It runs a workflow repeatedly for real and writes out a report, with every number in that
report actually measured, nothing estimated.

```bash
kairo bench run checkout-settlement --warmups 1 --repetitions 10
kairo bench list
kairo bench show <report-file>
```

You can also measure real crash recovery, not just timing:

```bash
kairo bench run checkout-settlement --failure-scenario worker-kill --repetitions 3
```

Each repetition starts a fresh local runtime, submits the run, waits for a worker to pick it up,
kills that worker for real, and measures how long another worker actually takes to recover.

## Checking your setup

```bash
kairo doctor
kairo storage check
kairo storage check --input ./some-file
```

`kairo doctor` tells you what is missing or set up wrong and how to fix it. `kairo storage check`
confirms your storage backend can actually be written to and read from. Add `--input <path>` to
send a real file through it and make sure you get back exactly what you put in.

To use Cloudflare R2 for storage instead of local files, set `KAIRO_R2_ACCOUNT_ID`,
`KAIRO_ARTIFACT_BUCKET`, `KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` in your
environment (or in a local `.env` file, which is never committed to git), then run `kairo init
--r2`. Kairo never writes credentials anywhere in source.

## Contributing

Before opening a pull request, make sure these all pass. CI runs the same checks:

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The codebase avoids unsafe code and panics in normal code paths, keeps files reasonably sized, and
prefers real tests over mocked ones.
