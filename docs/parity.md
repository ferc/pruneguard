# Parity Notes

`oxgraph` tracks parity against local reference corpora:

- `knip`
- `dependency-cruiser`
- `oxc`
- `claude-attack`

This harness is smoke-oriented today, not exact-output parity. The current goals are:

- no panic on real repositories
- valid JSON output
- stable ordering
- plausible inventory sizes
- no unexpected parity warnings

Known intentional differences:

- `oxgraph` is graph-first and proof-oriented, so finding IDs and evidence differ from both `knip` and `dependency-cruiser`
- `oxgraph` separates value and type liveness in unused-export reporting
- `oxgraph` treats `baseline.json` as a prior `AnalysisReport`, not as a dedicated baseline schema
- parity runs always use `--no-cache --no-baseline` to avoid cache/baseline noise
- partial-scope positional scans are explicitly advisory for dead-code findings; parity runs use full-repo scans
- findings now carry `confidence`, so parity review should consider both the finding count and whether high-confidence findings look plausible

Run the current real-repo harness with:

```sh
just parity
```
