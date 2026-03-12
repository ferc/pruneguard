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

- default path: `.oxgraph/cache.redb`
- disabled via `--no-cache`
- reused for extracted facts, parse diagnostics, resolutions, and manifest metadata
- deleted-path recovery for `--changed-since` consults the cache path index first

Warm runs should reduce extraction and resolution work without changing the final findings.

Build/runtime note:

- `pnpm -r build` uses the N-API packaging path through `@napi-rs/cli`
- `pnpm --dir apps/oxgraph build-js` and `cargo check -p oxgraph --features napi` validate the JS wrapper and Rust N-API surface separately in PR CI
- in restricted environments that block Cargo registry access, the Rust workspace and local JS smoke can still pass while the full N-API build step fails on dependency fetch
