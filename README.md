# Kairo

**Local when possible. Durable when necessary.**

Kairo runs workflows built out of WebAssembly Components. Think of a workflow as a small pipeline
of steps, each one a self-contained Component, wired together into a graph. Most of the time you
just want those steps to run fast, one after another, handing data straight to the next step with
no overhead. But sometimes a step's result is too important to lose — if a machine crashes
halfway through, you need to pick up exactly where you left off instead of starting over. Kairo
lets you mark which steps need that guarantee and leaves everything else fast and simple. You
describe the workflow; Kairo figures out where each step's data should live and recovers your
work automatically if something dies partway through.

## What you need

- Rust `1.95` or newer. Run `rustup show` to check what you have.
- Docker, but only if you want to try a local MinIO artifact store via `kairo init --minio`.
  Everything else works fine without it.

## Try it in under a minute

```bash
cargo install --path crates/cli --locked --root "$HOME/.local" --force
kairo init
kairo run checkout-settlement
kairo inspect
```

`kairo init` sets up local storage and a project folder for Kairo to work in. `kairo run
checkout-settlement` runs a small bundled example workflow start to finish. `kairo inspect` tells
you what just happened — which steps ran, how long each one took, and what got saved along the
way.

## The commands you'll actually use

| Command | What it does |
|---|---|
| `kairo run <workflow>` | Run a workflow |
| `kairo run <workflow> --watch` | Run it and watch progress live |
| `kairo run <workflow> --run <name>` | Run it as a named run you can come back to and resume |
| `kairo up --workers N` | Start a long-running service with N workers |
| `kairo down` | Stop that service |
| `kairo tui` | Open a dashboard for watching and running workflows |
| `kairo runs` | List the runs you've done locally (also `kairo cells`) |
| `kairo inspect [<run>]` | See details from a run; defaults to the most recent one |
| `kairo inspect <run> --export <path>` | Pull a run's saved result back out without rerunning it |
| `kairo inspect <run> --verify` | Double-check a run's saved checkpoints are actually intact |
| `kairo workflows [<path>]` | List the workflows Kairo knows about, or show one workflow's shape |
| `kairo signal <run> [<name>]` | Send a signal to a run that's waiting for one |
| `kairo cancel <run>` | Cancel a run that's queued, waiting, or in progress |
| `kairo prune [--older-than-hours N] [--workflow NAME]` | Clean up old, finished run records |
| `kairo chaos kill <worker>` | Kill a worker on purpose, to see how recovery holds up |
| `kairo bench run <workflow>` | Time real, repeated runs and save the results |
| `kairo doctor` | Check that your local setup is healthy, and explain what's wrong if it isn't |
| `kairo storage check [--input <path>]` | Confirm your storage backend actually works |

Add `--json` to get machine-readable output (supported by `runs` and `doctor` today) or `--quiet`
to skip the progress lines and just get the result — handy for scripts and CI.

## Example workflows to poke at

Run any of these with `kairo run <name>`:

- **video** — Looks at an MP4 or Y4M clip and reports frame count, dimensions, brightness, and
  how much it changes frame to frame. Handles up to 24 decoded frames; small clips only.
- **doc** — Counts lines, words, characters, and paragraphs in a text file or a DOCX file.
- **invoice** — Validates and totals up structured invoice records from a JSONL or CSV file.
- **redact** — Finds and blanks out one email-looking string or one 10-digit number in a piece of
  text.
- **preview** — Turns a short video clip into a grayscale contact-sheet image.
- **approval** — Pauses and waits for someone to send an approval signal before continuing.
- **delay** — Waits for a set amount of time without tying up a worker while it does.
- **order** — Calls an external "create order" action safely — running it twice never double-books.

Run `kairo workflows` any time to see the full list, along with what each one expects as input and
what it hands back.

## Keeping a service running

```bash
kairo up --workers 4
kairo run checkout-settlement
kairo tui
kairo down
```

`kairo up` starts a small local service with its own worker pool that stays running between
separate `kairo run` commands. With it running, workflows get properly scheduled, retried if
something goes wrong, and recovered instead of just running inline in your terminal.

## Building your own workflow

```bash
kairo workflow create
```

This walks you through it — what to call the workflow, which Components to chain together, what
order they run in, and what input they take. Add `--advanced` if you also want to configure
durability, waits, or outside calls.

## Runs you can come back to

Give a run a name whenever you think you'll need to find it again later — to send it a signal,
look at what happened, or cancel it:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo inspect approval-flow --verify
kairo cancel approval-flow
```

Timers don't tie up a worker while they wait. And if a workflow calls out to something external,
Kairo makes sure a retry after a crash never triggers that action twice.

## Running things close together

When you've got multiple workers going (`kairo up`) and a workflow has some steps that need to be
durable, Kairo may split the workflow into chunks and pin each chunk to a single worker, so the
steps inside it can talk to each other directly instead of going over the network. A chunk only
ever moves to another worker when there's a real, cheap handoff point and another worker is
genuinely free to take it — never just because it happens to be idle. This all happens on its own;
you never have to think about it. If a run's chunks did move around, `kairo inspect <run>` will
show you where.

## Cleaning up old runs

Every durable run leaves behind a small record on disk. Nothing deletes these for you, so if
you've been developing locally for a while, they can pile up:

```bash
kairo prune                 # see what would be removed
kairo prune --yes           # actually remove it
kairo prune --older-than-hours 24 --workflow checkout-settlement
```

`kairo prune` only ever touches runs that have actually finished — completed, failed, or
canceled. Anything still in progress is always left alone.

## Measuring performance

`kairo bench` is something you reach for on purpose — it never runs as part of a normal `kairo
run`. It runs a workflow repeatedly for real and writes out a report. Every number in that report
is something it actually measured; nothing is estimated or made up.

```bash
kairo bench run checkout-settlement --warmups 1 --repetitions 10
kairo bench run redact sample.txt --repetitions 5     # works for these too
kairo bench list
kairo bench show <report-file>
```

Reports get saved automatically and never overwrite something already there. If a run fails or
gets canceled during a benchmark, that's recorded separately and doesn't get folded into the
summary numbers.

You can also benchmark real recovery, not just timing:

```bash
kairo bench run checkout-settlement --failure-scenario worker-kill --repetitions 3
```

Each repetition starts a fresh service, submits the run, waits for a worker to pick it up, kills
that worker for real, and measures how long it actually takes another worker to recover — the
same mechanism behind `kairo chaos kill`. It takes over your local service while it runs.

## Checking your setup

```bash
kairo doctor
kairo storage check
kairo storage check --input ./some-file
kairo workers
```

`kairo doctor` tells you what's missing or misconfigured and how to fix it. `kairo storage check`
confirms your storage backend can actually be written to and read from — add `--input <path>` to
round-trip a real file through it and make sure you get back exactly what you put in.

To use Cloudflare R2 for storage, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`,
`KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` in your environment (or a local `.env`
file, which is never committed), then run `kairo init --r2`. Kairo never writes credentials
anywhere in source.

## Contributing

Before opening a pull request, make sure these all pass — CI runs the same checks:

```bash
cargo build --workspace
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The codebase avoids `unsafe` code and panics in production paths, keeps files reasonably sized,
and expects real tests over mocked ones.
