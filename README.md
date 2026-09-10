# Kairo

Kairo runs reliable WebAssembly Component workflows: local and fast when it
can be, durable where recovery needs it.

## Start here

```bash
cargo install --path crates/cli --locked --root "$HOME/.local" --force
kairo init
kairo run checkout-settlement
kairo inspect
```

`init` creates project-local state and verifies local artifact storage. `run`
accepts a workflow path or an unambiguous declared workflow name, so the
checkout example resolves from `demos/checkout/workflow.yaml`.

Watch one execution without opening the dashboard:

```bash
kairo run checkout-settlement --watch
```

For a persistent local runtime:

```bash
kairo up --workers 4
kairo run checkout-settlement
kairo tui
kairo down
```

`kairo start --workers N` and `kairo stop` remain available for scripts and
operations. `tui` is the multi-run dashboard; `inspect` is the detailed
post-run view.

## Reference workflows

List the copyable workflows included with Kairo:

```bash
kairo workflows
kairo run video
kairo run video ./my-video.y4m --watch
kairo run doc ./my-records.csv
kairo run delay
```

`video` validates Y4M content and reports frame count and average luma. `doc`
validates and aggregates structured invoice records in JSONL or CSV. Omit the
file to use the small bundled input. `--input-file` remains available for
scripts, and the old longer workflow names remain aliases.

These are local stream workflows: input stays local and is processed with
bounded batches. Kairo records its logical name, size metrics, and SHA-256
identity for `inspect` and the TUI, but does not make the file durable or claim
multi-worker recovery for it. They are ordinary workflow YAML files under
`demos/reference/`.

The durable examples start local workers automatically when needed:

```bash
kairo run approval
kairo signal approval
kairo inspect

kairo run order
kairo inspect
```

`approval` returns once it is safely waiting for `approval.granted`, so the
signal can be sent from the same terminal. If multiple approval runs are
waiting, Kairo asks for an explicit run name rather than guessing. `order` records
the `create-order` action through Kairo's local idempotent effect provider.
For recovery practice, keep a durable run active and use `kairo chaos kill
worker-1`; inspect and the TUI show the actual recovered run state.

## Create a workflow

```bash
kairo workflow create
```

The default guided flow asks only for a name, Components, their order, and an
input. It validates Components and previews the graph before saving. Use
`--advanced` to be prompted for durability, waits, and effects, or provide
those options directly in scripts:

```bash
kairo workflow create --name approval-flow \
  --component first.wat --component second.wat \
  --durability required --wait signal:approval.granted --effect record-order
```

Generated workflows are scalar and linear. YAML remains the full declarative
format for stream and advanced graph workflows. `kairo new workflow ...`
remains as the single-Component shortcut.

## Durable workflows

Named runs are only needed when you need to refer to one exact execution:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo inspect approval-flow --verify
```

The short signal form infers the one pending signal; scripts can continue to
use `kairo signal RUN SIGNAL`. Timers wait without occupying a worker. Effects
use durable receipts and idempotency keys; inspect shows their committed state.

For recovery testing, use `kairo chaos kill worker-1` against a persistent
runtime. Kairo waits for safe ownership takeover, then recovers only from a
valid durable boundary.

## Diagnose and configure

```bash
kairo doctor
kairo storage check
kairo workers
```

For Cloudflare R2, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`,
`KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` in your environment
or local `.env`, then run `kairo init --r2`. Kairo never writes credentials.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```
