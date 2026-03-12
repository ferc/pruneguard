# Parity Notes

`oxgraph` tracks parity against three local reference corpora:

- `knip`
- `dependency-cruiser`
- `oxc`

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
- `oxgraph` currently supports a subset of the long-term rule filter model

Run the current real-repo harness with:

```sh
just parity
```
