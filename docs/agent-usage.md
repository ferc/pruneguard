# Agent Usage

This document describes how to consume pruneguard from an AI coding agent or
automated pipeline. Every command produces structured JSON output when invoked
with `--format json`. All JS API functions return typed objects.

## Install

```sh
npm install pruneguard
```

No Rust toolchain required. The package ships a pre-built native binary for
each supported platform. The JS API spawns this binary and parses the result.

## JS API

```js
import {
  scan,
  review,
  safeDelete,
  fixPlan,
  impact,
  explain,
  suggestRules,
  run,
  binaryPath,
  loadConfig,
  schemaPath,
} from "pruneguard";
```

### Error contract

All functions throw `PruneguardExecutionError` with one of these codes:

| Code | Meaning |
|---|---|
| `PRUNEGUARD_BINARY_NOT_FOUND` | Native binary not found for this platform |
| `PRUNEGUARD_EXECUTION_FAILED` | Binary exited with unexpected code |
| `PRUNEGUARD_JSON_PARSE_FAILED` | Binary output was not valid JSON |

The error object also carries `exitCode`, `stdout`, `stderr`, and `args`
fields for diagnostics.

## Commands

### scan

Get inventories, findings, and proof-friendly evidence.

```sh
pruneguard --format json scan
```

```js
const report = await scan({ cwd: "/path/to/repo" });

// report.findings: Array<Finding>
// report.summary: { totalFindings, errors, warnings, infos }
// report.stats: { durationMs, unresolvedSpecifiers, confidenceCounts, ... }
// report.inventories: { files, packages, workspaces }
// report.entrypoints: Array<{ path, kind, profile, source }>
```

### review

Branch gate: classifies findings as blocking vs advisory with trust summary.
Exit 0 = safe to merge. Exit 1 = blocking findings exist.

```sh
pruneguard --changed-since origin/main --format json review
```

```js
const result = await review({ baseRef: "origin/main", noCache: true });

// result.blockingFindings: high-confidence error/warn findings
// result.advisoryFindings: lower-confidence or info findings
// result.trust: { fullScope, baselineApplied, unresolvedPressure, confidenceCounts }
// result.recommendations: string[]
// result.proposedActions: structured remediation steps
```

Check `blockingFindings` array. If empty, the branch is clean. The `trust`
object reports `fullScope`, `baselineApplied`, and `unresolvedPressure`.

### safe-delete

Evaluates targets for safe deletion: safe / needsReview / blocked. Returns
confidence levels, reasons, and deletion order.

```sh
pruneguard --format json safe-delete src/old.ts src/legacy/widget.ts
```

```js
const result = await safeDelete({ targets: ["src/old.ts", "src/legacy/widget.ts"] });

// result.safe: Array<{ target, confidence, reasons }>
// result.needsReview: Array<{ target, confidence, reasons }>
// result.blocked: Array<{ target, confidence, reasons }>
// result.deletionOrder: string[]
```

Check `safe` for targets that can be deleted immediately. Check `blocked`
for targets that must not be deleted. Follow `deletionOrder` for the
recommended sequence.

### fix-plan

Generate a structured remediation plan with specific actions per finding.

```sh
pruneguard --format json fix-plan src/old.ts
```

```js
const plan = await fixPlan({ targets: ["src/old.ts"] });

// plan.actions: Array<{ id, kind, targets, why, steps, risk, confidence }>
// plan.blockedBy: string[]
// plan.verificationSteps: string[]
// plan.riskLevel: "low" | "medium" | "high"
```

### impact

Estimate blast radius before edits.

```sh
pruneguard --format json impact src/utils/helpers.ts
```

```js
const blast = await impact({ target: "src/utils/helpers.ts" });

// blast.affectedEntrypoints: string[]
// blast.affectedPackages: string[]
// blast.affectedFiles: string[]
```

### explain

Understand why something is live, unused, or violating a boundary.

```sh
pruneguard --format json explain src/old.ts#deprecatedFn
```

```js
const proof = await explain({ query: "src/old.ts#deprecatedFn" });

// proof.proofs: recursive tree of { node, relationship, children }
// proof.relatedFindings: Array<Finding>
// proof.queryKind: "finding" | "file" | "export"
```

### suggest-rules

Auto-suggest governance rules based on graph analysis.

```sh
pruneguard --format json suggest-rules
```

```js
const rules = await suggestRules();

// rules.suggestedRules: Array<{ kind, name, description, configFragment, confidence }>
// rules.tags: Array<{ name, glob, rationale }>
// rules.ownershipHints: Array<{ pathGlob, suggestedOwner, rationale }>
// rules.hotspots: Array<{ file, crossPackageImports, suggestion }>
```

### run (escape hatch)

Run arbitrary CLI args when you need flags not covered by the typed API.

```js
const result = await run(["--format", "json", "--daemon", "off", "scan"]);

// result.exitCode: number
// result.stdout: string
// result.stderr: string
// result.durationMs: number
// result.args: string[]
```

### binaryPath

Resolve the native binary path for custom integrations.

```js
console.log(binaryPath());
// => /path/to/node_modules/@pruneguard/cli-darwin-arm64/bin/pruneguard
```

## Workflows

### Branch review (CI gate)

1. Run `review` with `--changed-since origin/main`.
2. If `blockingFindings` is empty, the branch is clean.
3. If not, report each blocking finding and fail the build.
4. Optionally surface `advisoryFindings` as non-blocking annotations.

### Safe deletion

1. Identify candidate files from scan findings or manual selection.
2. Run `safeDelete` on the candidates.
3. Delete files in `safe` immediately.
4. Flag `needsReview` for human attention.
5. Never delete files in `blocked`.
6. Follow `deletionOrder` for the recommended sequence.

### Fix-plan repair loop

1. Run `fixPlan` on target findings.
2. Execute each action's `steps` programmatically.
3. Re-scan with `--no-cache` to verify the fix.
4. Repeat until `findings.length === 0` or only blocked items remain.

### Manual investigation

1. Run `scan --format json`.
2. Inspect `unused-file` / `unused-export` findings.
3. Run `impact` on candidate removals.
4. Run `explain` on anything unclear.

### Baseline workflow

1. Save a `scan --format json --no-cache --no-baseline` result as
   `baseline.json` on the main branch.
2. On feature branches, compare current scan against baseline.
3. Only surface new findings that are not in the baseline.
