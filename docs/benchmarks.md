# Benchmarks

## Target Latency

These are the target latencies for pruneguard across repository size tiers.

### Warm Daemon

| Repo size                                 | `review`  | `impact` / `explain` |
|-------------------------------------------|-----------|----------------------|
| Small repos (< 100 files)                 | < 10 ms   | < 5 ms               |
| Medium repos (knip/dependency-cruiser)     | < 50 ms   | < 20 ms              |
| Large repos (oxc scale, ~5000+ files)     | < 150 ms  | < 50 ms              |

### Cold One-Shot (full graph build, no cache)

| Corpus               | Target wall-clock |
|----------------------|-------------------|
| knip                 | < 1.5 s           |
| dependency-cruiser   | < 2.0 s           |
| oxc                  | < 6.0 s           |

Cold one-shot times include discovery, extraction, resolution, graph build,
analysis, and report rendering. These are the worst-case numbers for a first
run with `--no-cache --no-baseline`.

## Running Benchmarks

### Single corpus scan

```sh
just benchmark CASE=../../path/to/repo
```

Runs a release-mode scan with `--format json --no-cache --no-baseline` on the
given repository. The `durationMs` field in the JSON output is the primary
measurement.

### All configured corpora

```sh
just benchmark-repos
```

Runs the configured real-repo parity suite in release mode. This exercises
knip, dependency-cruiser, oxc, and other configured corpora.

### Microbenchmarks (Rust criterion)

```sh
just benchmark-workspace
```

Runs the Rust criterion microbenchmark suite across all crates in the
workspace.

### Fixture scans

```sh
just fixture namespace-imports
```

Runs a single fixture case through the scan smoke test.

## Interpreting Results

The JSON report includes a `stats` object with timing and graph size metrics:

```json
{
  "stats": {
    "durationMs": 42,
    "filesParsed": 1200,
    "filesCached": 800,
    "graphNodes": 1500,
    "graphEdges": 4200,
    "unresolvedSpecifiers": 3,
    "executionMode": "oneshot"
  }
}
```

Key fields for benchmarking:

- `durationMs` -- total wall-clock time for the analysis
- `filesParsed` -- files that required fresh parsing (cache miss)
- `filesCached` -- files served from cache (cache hit)
- `graphNodes` / `graphEdges` -- graph size, correlates with analysis cost
- `cacheHits` / `cacheMisses` -- cache effectiveness
- `executionMode` -- "oneshot" or "daemon"

### Cold vs Warm

To measure cold performance, always pass `--no-cache`:

```sh
cargo run --release -p pruneguard -- --format json --no-cache --no-baseline scan ../../target-repo
```

To measure warm performance, run twice and use the second run's timing:

```sh
cargo run --release -p pruneguard -- --format json --no-baseline scan ../../target-repo
cargo run --release -p pruneguard -- --format json --no-baseline scan ../../target-repo
```

## CI Benchmark Tracking

Automated benchmark tracking in CI is planned. When available, the workflow
will:

1. Run the parity corpus suite on every push to `main`
2. Record `durationMs`, graph size, and cache metrics
3. Fail the build if any corpus regresses beyond a configured threshold
4. Publish trend data to a tracking dashboard
