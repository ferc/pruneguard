# Performance

## Runtime model

pruneguard builds a full module graph on every invocation. The graph is built
once and reused by all analyzers in the same run. This architecture favors
accuracy (one consistent graph) over incremental shortcuts that could produce
stale results.

The binary is compiled Rust, powered by [oxc](https://oxc.rs) for parsing
and resolution. No V8 or Node.js runtime is involved in the hot path.

### Execution modes

| Mode | Latency profile | Use case |
|---|---|---|
| **Daemon** (default local) | Sub-millisecond for `review`, `impact`, `explain` after initial warm-up | Local development, agent workflows |
| **One-shot** (default CI) | Full graph build per invocation | CI pipelines, benchmarks, deterministic runs |

## What affects performance

| Factor | Impact | Knobs |
|---|---|---|
| Repository size (files) | Linear in graph build time | `--focus` (post-analysis filter, no speedup) |
| Cache | Reduces extraction and resolution work | `--no-cache` to disable |
| Partial-scope paths | `scan <paths...>` can reduce work | Dead-code findings become advisory |
| Baseline | Minimal cost (filtering after analysis) | `--no-baseline` to disable |
| `--changed-since` | Full graph still builds; filtering is post-analysis | Use for output filtering, not speedup |
| Daemon | Amortizes graph build across multiple queries | `--daemon auto` (default) |

## Cache behavior

- Default path: `.pruneguard/cache.redb`
- Disabled via `--no-cache`
- Stores: extracted facts, parse diagnostics, resolutions, manifest metadata
- Does not store: final findings (always recomputed)
- Warm runs reduce extraction and resolution work without changing results

Key cache stats in the JSON report:

```json
{
  "stats": {
    "filesParsed": 200,
    "filesCached": 800,
    "cacheHits": 800,
    "cacheMisses": 200,
    "cacheEntriesRead": 1000,
    "cacheEntriesWritten": 200
  }
}
```

## Measuring performance

### Single corpus scan

```sh
just benchmark ../../path/to/repo
```

Runs a release-mode scan with `--format json --no-cache --no-baseline` on the
given repository. The `durationMs` field in the JSON output is the primary
measurement.

### All configured corpora

```sh
just benchmark-repos
```

Runs the configured real-repo parity suite in release mode.

### Microbenchmarks (Rust criterion)

```sh
just benchmark-workspace
```

### Cold vs warm

To measure cold performance, always pass `--no-cache`:

```sh
cargo run --release -p pruneguard -- --format json --no-cache --no-baseline scan ../../target-repo
```

To measure warm performance, run twice and use the second run's timing:

```sh
cargo run --release -p pruneguard -- --format json --no-baseline scan ../../target-repo
cargo run --release -p pruneguard -- --format json --no-baseline scan ../../target-repo
```

### Interpreting results

The JSON report includes a `stats` object with timing and graph size metrics:

```json
{
  "stats": {
    "durationMs": 42,
    "filesParsed": 1200,
    "filesCached": 800,
    "graphNodes": 1500,
    "graphEdges": 4200,
    "unresolvedSpecifiers": 3
  }
}
```

- `durationMs` -- total wall-clock time
- `filesParsed` -- files that required fresh parsing (cache miss)
- `filesCached` -- files served from cache (cache hit)
- `graphNodes` / `graphEdges` -- graph size, correlates with analysis cost
- `executionMode` -- "oneshot" or "daemon"
- `indexWarm` / `indexAgeMs` -- daemon index status
