# Contributing

## Clippy Lint Policy

CI enforces `cargo clippy --workspace -- --deny warnings`. All new code must
pass without warnings.

### When `#[allow(clippy::...)]` is acceptable

Suppressions must be **local** (`#[allow(...)]` on the item, not crate-wide)
unless the lint is inherent to the crate's purpose (e.g. `print_stdout` in the
CLI binary).

Approved categories:

| Category | Example lints | Approved locations |
|---|---|---|
| CLI boundary output | `print_stdout`, `print_stderr` | `apps/pruneguard/src/main.rs` (crate-level) |
| Schema/report shape | `struct_excessive_bools`, `struct_field_names` | Report and schema structs |
| Readability-oriented parsing | `naive_bytecount`, `while_let_on_iterator` | Parser/extractor code where the pattern is intentionally clearer |
| Display-only precision | `cast_precision_loss` | Formatting percentages for human display (line-level only) |
| Orchestrator length | `too_many_lines` | Command routers, analysis pipelines, and graph assembly where splitting would hide control flow |
| Constructor helpers | `too_many_arguments` | Functions where params map 1:1 to struct fields |
| Test readability | `similar_names` | Test files |

### What is NOT acceptable

- Suppressions covering stubs or placeholders
- Broad crate-wide suppressions (except CLI output lints in the CLI binary)
- `unsafe` in project code unless explicitly reviewed and documented
- New `cast_possible_truncation` — use `u32::try_from` with an explicit
  `expect` message instead

### CI guard

The `scripts/lint_audit.sh` script tracks the total count of Clippy
suppressions. If a PR increases the count beyond the recorded baseline, CI
fails. To update the baseline after a justified addition, run:

```sh
./scripts/lint_audit.sh --update
```
