# Reference workflows

These are ordinary Kairo workflows. Their YAML, Components, and bounded inputs
can be copied and changed without a special runtime path.

| Workflow | Input | Expected result | Kairo behavior |
| --- | --- | --- | --- |
| `video-processing` | `sample.y4m` | 1 frame, average luma 66 | three Components with bounded streams |
| `document-processing` | `records.jsonl` | 3 records, total 6350 cents | JSONL validation and aggregation across bounded streams |
| `approval` | scalar 2500 | 3154 | checkpoint, durable signal wait, then resume |
| `delayed-processing` | scalar 2500 | 3498 | checkpoint, durable timer, then resume |
| `order-processing` | scalar 2500 | 2655 | checkpoint and idempotent `create-order` action |

Start with:

```bash
kairo workflows
kairo run document-processing --watch
kairo inspect
```

Use `--input-file PATH` to replace a stream workflow's checked-in input. The
approval workflow returns after entering its durable wait, so it can be resumed
from the same terminal:

```bash
kairo run approval --run invoice-approval
kairo signal invoice-approval
kairo inspect invoice-approval
```
