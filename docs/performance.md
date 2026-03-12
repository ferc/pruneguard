# Performance

Current performance model:

- full graph build per run
- changed-since narrows findings after graph build
- baseline suppresses findings after analysis
- cache stores substrate facts, not final findings
- `--focus` is post-analysis filtering and does not shrink graph construction
- positional `scan <paths...>` can reduce work, but dead-code results are advisory in that mode
- `--no-baseline` is recommended for deterministic parity and benchmark runs

Cache behavior:

- default path: `.pruneguard/cache.redb`
- disabled via `--no-cache`
- reused for extracted facts, parse diagnostics, resolutions, and manifest metadata
- deleted-path recovery for `--changed-since` consults the cache path index first

Warm runs should reduce extraction and resolution work without changing the final findings.

Build/runtime note:

- the supported runtime model is binary-backed: the `pruneguard` npm package includes a JS wrapper that spawns the shipped Rust binary
- `pnpm --dir apps/pruneguard build-js` builds the JS wrapper
- `just pack-smoke` validates the full package install contract
- `just stage-release` produces publishable packages in `.release/npm/`
