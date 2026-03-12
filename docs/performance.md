# Performance

Current performance model:

- full graph build per run
- changed-since narrows findings after graph build
- baseline suppresses findings after analysis
- cache stores substrate facts, not final findings

Cache behavior:

- default path: `.oxgraph/cache.redb`
- disabled via `--no-cache`
- reused for extracted facts, parse diagnostics, resolutions, and manifest metadata

Warm runs should reduce extraction and resolution work without changing the final findings.
