# Operational

Operational defaults:

- cache path: `.pruneguard/cache.redb`
- baseline search order:
  - config directory `baseline.json`
  - project root `baseline.json`
- baseline can be disabled with `--no-baseline`
- changed-since model:
  - full graph build
  - affected-scope filtering after analysis
  - deleted paths fall back to broader findings if recovery is incomplete
- focus filtering:
  - full graph still builds
  - only returned findings/proofs/impact sets are narrowed
- partial scan paths:
  - `scan <paths...>` narrows the analyzed file set
  - dead-code findings from partial-scope scans are advisory
  - `--require-full-scope` turns advisory partial scans into an exit-code `2` failure
- trust hints:
  - findings include `confidence`
  - reports include unresolved-specifier counts by reason
  - reports include `resolvedViaExports`

Useful commands:

```sh
just ready
just schemas
just benchmark ../../claude-attack
just benchmark-repos
just parity
just smoke-repos
pnpm --dir apps/pruneguard build-js
just stage-release
just pack-smoke
```
