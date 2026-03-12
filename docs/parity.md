# Parity Notes

`pruneguard` tracks parity against local reference corpora:

- `knip`
- `dependency-cruiser`
- `oxc`
- `claude-attack`

This harness is smoke-oriented today, not exact-output parity. The tightened parity expectations are:

- no panic on any real repository or fixture
- valid JSON output on every run (stdout must parse as JSON)
- deterministic ordering of findings, inventories, and entrypoints across repeated runs
- minimum inventory sizes per corpus (file counts must not regress below known thresholds)
- no unexpected parityWarnings in the report
- no trust summary regressions on known corpora (confidence distribution must not shift toward lower tiers without explanation)

Known intentional differences:

- `pruneguard` is graph-first and proof-oriented, so finding IDs and evidence differ from both `knip` and `dependency-cruiser`
- `pruneguard` separates value and type liveness in unused-export reporting
- `pruneguard` treats `baseline.json` as a prior `AnalysisReport`, not as a dedicated baseline schema
- parity runs always use `--no-cache --no-baseline` to avoid cache/baseline noise
- partial-scope positional scans are explicitly advisory for dead-code findings; parity runs use full-repo scans
- findings now carry `confidence`, so parity review should consider both the finding count and whether high-confidence findings look plausible
- `pruneguard` classifies `node:`, `bun:`, `deno:` prefixed specifiers as externalized rather than unresolved
- `pruneguard` excludes ambient declaration files (`.d.ts`, `.d.mts`, `.d.cts`) from dead-code findings
- `pruneguard` checks package.json scripts for direct dependency usage before reporting unused dependencies
- confidence scoring uses three tiers (high, medium, low) to indicate finding trustworthiness

Run the current real-repo harness with:

```sh
just parity
```
