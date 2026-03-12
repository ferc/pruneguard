# Agent Usage

Useful agent workflows:

- `scan --format json`
  - get inventories, findings, and proof-friendly evidence
- `impact <target>`
  - estimate blast radius before edits
- `explain <finding-id|path>`
  - understand why something is live, unused, or violating a boundary
- baseline workflow
  - save a prior `scan --format json` as `baseline.json`
  - use later scans to focus on new findings

Typical deletion flow:

1. run `scan --format json`
2. inspect `unused-file` / `unused-export`
3. run `impact` on candidate removals
4. run `explain` on anything unclear
