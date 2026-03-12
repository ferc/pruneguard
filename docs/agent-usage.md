# Agent Usage

## Commands

- `scan --format json`
  - get inventories, findings, and proof-friendly evidence
- `review --changed-since origin/main --format json`
  - branch gate: classifies findings as blocking vs advisory with trust summary
  - exit 0 = safe to merge, exit 1 = blocking findings exist
- `safe-delete <targets...> --format json`
  - evaluates targets for safe deletion: safe / needsReview / blocked
  - returns confidence levels, reasons, and deletion order
- `impact <target>`
  - estimate blast radius before edits
- `explain <finding-id|path>`
  - understand why something is live, unused, or violating a boundary

## Workflows

### Branch review (CI gate)

```sh
pruneguard --changed-since origin/main --format json review
```

Check `blockingFindings` array. If empty, the branch is clean.
The `trust` object reports `fullScope`, `baselineApplied`, and `unresolvedPressure`.

### Safe deletion

```sh
pruneguard --format json safe-delete src/old.ts src/legacy/widget.ts
```

Check `safe` array for targets that can be deleted immediately.
Check `blocked` for targets that must not be deleted.
Follow `deletionOrder` for the recommended sequence.

### Manual investigation

1. run `scan --format json`
2. inspect `unused-file` / `unused-export` findings
3. run `impact` on candidate removals
4. run `explain` on anything unclear

### Baseline workflow

- save a prior `scan --format json` as `baseline.json`
- use later scans to focus on new findings only
