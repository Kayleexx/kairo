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
kairo --verbose run demos/checkout/workflow.yaml --state
```

The checkout workflow runs four Components and returns `3207`. The required
edge stores `2708` as a hashed artifact before the final shipping Component.
`storage check` writes and immediately reads a reusable check artifact through
the active backend; its hash proves that round trip without exposing storage
configuration.

Inspect the SQLite Cell journal, then run the same command again:

```bash
sqlite3 -header -column .kairo/checkout-settlement.db \
  'SELECT sequence, kind, step_index, output_value, artifact_hash FROM events;'

kairo --verbose run demos/checkout/workflow.yaml --state
```

The second run reports `resumed=true` and returns `3207`. SQLite records local
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
kairo --verbose run demos/invoice/workflow.yaml --state
```

It runs `10000 → 8500 → 9180 → 9679`, checkpoints the subtotal in R2, and
returns `9804` after the final service-fee Component.

```bash
sqlite3 -header -column .kairo/invoice-total.db \
  'SELECT sequence, kind, step_index, output_value, artifact_hash FROM events;'

kairo --verbose run demos/invoice/workflow.yaml --state
```

## Storage and recovery

`kairo init` selects one active user-wide backend. Choosing another backend
affects future checkpoints only; it never moves or removes existing artifacts
or SQLite journals. New local setup resolves `.kairo/artifacts` from the
current project, alongside its default `.kairo/<workflow>.db` journal.

If a workflow changes, Kairo refuses to reuse its existing journal. Use a new
`--state` path or archive that journal before starting the changed workflow.

`kairo init --minio` and `kairo init --endpoint URL --bucket NAME` remain
available for optional S3-compatible storage.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```
