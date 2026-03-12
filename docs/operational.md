# Operational Reference

## Runtime model

pruneguard ships as a compiled Rust binary. The `pruneguard` npm package
includes a JS wrapper that spawns the binary and parses its output. No Rust
toolchain is required to use pruneguard.

The binary resolution order is:

1. `PRUNEGUARD_BINARY` environment variable
2. Platform-specific npm package (e.g. `@pruneguard/cli-darwin-arm64`)
3. Development fallback (`target/release/pruneguard` or `target/debug/pruneguard`)
4. `PATH` lookup (disabled by default, enabled with `allowPathFallback`)

### Execution modes

| Mode | When | Behavior |
|---|---|---|
| **Daemon** (default local) | `--daemon auto` or `--daemon required` | Keeps graph warm in memory, sub-ms `review`/`impact`/`explain` |
| **One-shot** (default CI) | `--daemon off` or CI environment | Fresh graph build per invocation, deterministic |

The default `--daemon auto` uses the daemon when available and falls back
to one-shot if the daemon is not running. In detected CI environments
(e.g. `CI=true`), the default is one-shot.

## Cache

- Default path: `.pruneguard/cache.redb`
- Disable with `--no-cache`
- Stores extracted facts, parse diagnostics, resolutions, and manifest metadata
- Does not store final findings -- those are always recomputed
- Warm runs reduce extraction and resolution work without changing results
- Deleted-path recovery for `--changed-since` consults the cache path index

## Baseline

- Search order:
  1. Config directory `baseline.json`
  2. Project root `baseline.json`
- Disable with `--no-baseline`
- The baseline is a prior `AnalysisReport` (the same JSON shape as `scan` output)
- Suppresses findings that already existed in the baseline
- Use `--no-baseline` for deterministic CI, parity checks, and benchmarks

## Changed-since model

- Full graph is always built (even with `--changed-since`)
- Affected-scope filtering happens after analysis
- Deleted paths fall back to broader findings if recovery is incomplete
- Stats include `changedFiles`, `affectedFiles`, `affectedPackages`,
  `affectedEntrypoints`, and `affectedScopeIncomplete`

## Focus filtering

- `--focus <glob>` filters reported findings after full analysis
- The full graph still builds; only the output is narrowed
- Stats include `focusApplied`, `focusedFiles`, `focusedFindings`

## Partial-scope scans

- `scan <paths...>` narrows the analyzed file set
- Dead-code findings from partial-scope scans are marked advisory
- `--require-full-scope` turns advisory partial scans into exit code 2
- Stats include `partialScope` and `partialScopeReason`

## Trust hints

- Every finding carries `confidence` (high, medium, low)
- Reports include `unresolvedSpecifiers` count and `unresolvedByReason` breakdown
- Reports include `resolvedViaExports` count
- Reports include `confidenceCounts` for aggregate trust assessment
- The `trust` object in `review` output summarizes scope and pressure metrics

## Development commands

```sh
just ready               # fmt + check + test + lint
just build               # Release binary
just build-js            # Build the JS wrapper
just stage-release       # Stage npm packages into .release/
just pack-smoke          # End-to-end package install smoke test
just smoke-repos         # Opt-in real-repo smoke tests
just parity              # Real-repo parity checks
just benchmark CASE=path # Benchmark a single corpus
just benchmark-repos     # Benchmark all configured corpora
just schemas             # Regenerate shipped schemas
just schemas-check       # Verify schemas are committed
just ci                  # Full CI pipeline locally
```
