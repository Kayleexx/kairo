# Kairo

Kairo runs WebAssembly Component workflows locally and persists only explicit
recovery boundaries. It targets WASI 0.3 and the Component Model.

## Install

```bash
cargo install --path crates/cli --locked --root ~/.local --force
kairo --help
```

## Local durability walkthrough

Select local artifact storage from the project directory:

```bash
kairo init
# press Enter for local

kairo storage check
kairo --verbose run demos/checkout/workflow.yaml --cell order-1042
kairo workflows demos/checkout/workflow.yaml
kairo workflows
kairo cells checkout-settlement
kairo inspect order-1042 --verify
```

The checkout workflow runs four Components and returns `3207`. The required
edge stores `2708` as a hashed artifact before the final shipping Component.
`storage check` writes and immediately reads a reusable check artifact through
the active backend; its hash proves that round trip without exposing storage
configuration.

`workflows <file>` shows a validated graph and its recovery boundary; the
pathless form lists workflows observed in local Cells. `cells [workflow]`
lists local SQLite executions, while `inspect` shows component states,
durations, hashes, checkpoints, and the journal path. `--verify` checks every
recorded checkpoint against the active artifact backend.

```bash
kairo --verbose run demos/checkout/workflow.yaml --cell order-1042
```

The second run reports that it restored from the journal and returns `3207`.
SQLite records local
execution progress; the required-edge hash records the durable checkpoint.

## R2 durability walkthrough

Keep R2 credentials in ignored `.env`:

```env
KAIRO_R2_ACCOUNT_ID=...
KAIRO_ARTIFACT_BUCKET=...
KAIRO_R2_ACCESS_KEY_ID=...
KAIRO_R2_SECRET_ACCESS_KEY=...
```

Then select R2 and run the independent invoice workflow:

```bash
kairo init
# type r2

kairo storage check
kairo --verbose run demos/invoice/workflow.yaml --cell invoice-2026-001
kairo workflows demos/invoice/workflow.yaml
kairo cells invoice-total
kairo inspect invoice-2026-001 --verify
```

It runs `10000 → 8500 → 9180 → 9679`, checkpoints the subtotal in R2, and
returns `9804` after the final service-fee Component.

Stream workflows remain stateless and report their byte count, checksum, and
duration directly:

```bash
kairo workflows demos/stream/workflow.yaml
kairo run demos/stream/workflow.yaml
```

## Storage and recovery

`kairo init` selects the artifact backend. Cells always remain local SQLite
journals; local or R2 storage holds only required-edge checkpoints. Kairo
records the backend with each new checkpoint and rejects recovery through a
different backend. It never moves or removes existing artifacts or journals.

Use `--cell <id>` for normal runs and distinct IDs for concurrent or repeated
executions. `--state [file]` remains available for explicit journal paths. If
a workflow changes, Kairo refuses to reuse its existing Cell ID.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```
