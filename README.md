# Kairo

**Local when possible. Durable when necessary.**

Kairo runs reliable WebAssembly Component workflows: local and fast when possible, durable where recovery needs it.

## Quick start

```bash
cargo install --path crates/cli --locked --root "$HOME/.local" --force
kairo init
kairo run checkout-settlement
kairo inspect
```

## Core commands

| Command | Description |
|---|---|
| `kairo run <workflow>` | Run a workflow (scalar or stream) |
| `kairo run <workflow> --watch` | Live-watch a workflow run |
| `kairo up --workers N` | Start persistent service with N workers |
| `kairo down` | Stop the service |
| `kairo tui` | Multi-run terminal dashboard |
| `kairo inspect` | Post-run detail view |
| `kairo workflows` | List observed workflows |
| `kairo signal <id>` | Send a signal to a waiting run |
| `kairo chaos kill <worker>` | Kill a worker for recovery testing |
| `kairo doctor` | Check local setup |
| `kairo storage check` | Verify artifact store |

## Reference workflows

Included demos (use `kairo run <name>`):

- **video** - Analyze H.264/AVC MP4 or Y4M: frame count, dimensions, average luma, frame-to-frame luma change. Max 24 decoded 8-bit 4:2:0 frames; audio ignored; MP4 limited to 6 MiB and 1280x720.
- **doc** - Count lines, words, characters, paragraphs, longest line for UTF-8 text or extracted DOCX text.
- **invoice** - Validate and aggregate structured invoice records in JSONL or CSV.
- **doc redact** - Redact one ASCII email-like token or one 10-digit token at a time; normalizes CRLF to LF.
- **doc preview** - Grayscale contact-sheet PNG from up to 24 decoded frames of bounded H.264/AVC MP4.
- **approval** - Wait for `approval.granted` signal; use `kairo signal <id>` to resume.
- **delay** - Durable timer wait; worker released while waiting.
- **order** - Idempotent `create-order` external effect.

## Persistent runtime

```bash
kairo up --workers 4
kairo run checkout-settlement
kairo tui
kairo down
```

## Create a workflow

```bash
kairo workflow create
```

Guided prompt asks for name, components, order, and input. Use `--advanced` for durability, waits, and effects.

## Durable workflows

Named runs are only needed when referring to a specific execution:

```bash
kairo run demos/approval/workflow.yaml --run approval-flow
kairo signal approval-flow
kairo inspect approval-flow --verify
```

Timers wait without occupying a worker. Effects use durable receipts and idempotency keys.

## Diagnose and configure

```bash
kairo doctor
kairo storage check
kairo workers
```

For Cloudflare R2, set `KAIRO_R2_ACCOUNT_ID`, `KAIRO_ARTIFACT_BUCKET`, `KAIRO_R2_ACCESS_KEY_ID`, and `KAIRO_R2_SECRET_ACCESS_KEY` in environment or local `.env`, then run `kairo init --r2`. Kairo never writes credentials to source.

## Development

```bash
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```