# Agent Usage

This document describes how to consume pruneguard from an AI coding agent or
automated pipeline. The primary command is **`review`** -- running
`pruneguard` with no subcommand is equivalent to `pruneguard review`. It classifies
findings as blocking vs advisory and provides a trust summary that an agent
can use to decide whether to block a branch, flag for human review, or proceed.

Every command produces structured JSON output when invoked with `--format json`.
All JS API functions return typed objects.

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
  compatibilityReport,
  debugFrameworks,
  run,
  binaryPath,
  loadConfig,
  schemaPath,
  scanDot,
  migrateKnip,
  migrateDepcruise,
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

### compatibility-report

Check framework and toolchain compatibility. Surfaces unsupported signals
and trust downgrades that affect finding accuracy.

```sh
pruneguard --format json compatibility-report
```

```js
const compat = await compatibilityReport();

// compat.supportedFrameworks: string[]
// compat.heuristicFrameworks: string[]
// compat.unsupportedSignals: Array<{ signal, source, suggestion? }>
// compat.warnings: Array<{ code, message, affectedScope?, severity }>
// compat.trustDowngrades: Array<{ reason, scope, severity }>
```

Run this before acting on low-confidence findings. If `trustDowngrades` is
non-empty, some findings may be less reliable.

### debug frameworks

Show detailed framework detection diagnostics: which frameworks were
detected, what entrypoints and ignore patterns they contributed, and
which detections were heuristic.

```sh
pruneguard --format json debug frameworks
```

```js
const fwDebug = await debugFrameworks();

// fwDebug.detectedPacks: Array<{ name, confidence, signals, reasons }>
// fwDebug.allEntrypoints: Array<{ path, framework, kind, heuristic, reason }>
// fwDebug.allIgnorePatterns: string[]
// fwDebug.allClassificationRules: Array<{ pattern, classification }>
// fwDebug.heuristicDetections: string[]
```

### run (escape hatch)

Run arbitrary CLI args when you need flags not covered by the typed API.

```sh
pruneguard --format json --daemon off scan
```

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

```sh
# CLI: print the binary location
node -e "const { binaryPath } = require('pruneguard'); console.log(binaryPath())"
```

```js
import { binaryPath } from "pruneguard";

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

---

## Agent Decision Logic: review output

An AI agent should consume `review` output to decide whether to block a
branch, flag for human review, or proceed silently. Here is the recommended
decision tree.

```js
import { review } from "pruneguard";

const result = await review({ baseRef: "origin/main", noCache: true });

// Step 1: Check trust. If trust is degraded, findings may be unreliable.
if (result.trust.unresolvedPressure > 0.05) {
  // More than 5% of specifiers are unresolved. Findings are less trustworthy.
  // Flag for human review rather than auto-blocking.
  console.warn("High unresolved pressure -- findings may include false positives.");
}

if (!result.trust.fullScope) {
  // Partial-scope scan. Dead-code findings are advisory only.
  console.warn("Partial-scope scan -- dead-code findings are advisory.");
}

// Step 2: Check for blocking findings.
if (result.blockingFindings.length === 0) {
  // Branch is clean. Safe to merge.
  console.log("PASS: No blocking findings.");
  process.exit(0);
}

// Step 3: Report blocking findings.
for (const f of result.blockingFindings) {
  console.error(`BLOCK [${f.confidence}] ${f.code}: ${f.message}`);
  console.error(`  Subject: ${f.subject}`);
  if (f.suggestion) console.error(`  Fix: ${f.suggestion}`);
}

// Step 4: Optionally surface advisory findings as non-blocking annotations.
for (const f of result.advisoryFindings) {
  console.log(`ADVISORY [${f.confidence}] ${f.code}: ${f.message}`);
}

// Step 5: Use proposed actions to suggest fixes.
if (result.proposedActions) {
  for (const action of result.proposedActions) {
    console.log(`Suggested: ${action.kind} on ${action.targets.join(", ")}`);
  }
}

process.exit(1);
```

**Key decision points:**

| Condition | Agent action |
|-----------|-------------|
| `blockingFindings.length === 0` | Allow merge |
| `blockingFindings.length > 0` and `trust.fullScope` | Block merge, report findings |
| `blockingFindings.length > 0` and `!trust.fullScope` | Flag for human review (partial scope means less certainty) |
| `trust.unresolvedPressure > 0.05` | Add caveat that findings may be noisy |
| `trust.confidenceCounts.low > trust.confidenceCounts.high` | Suggest running `compatibilityReport` to diagnose trust issues |

---

## Agent Decision Logic: safe-delete output

When an agent needs to remove files (e.g., during a cleanup task or after
identifying unused code), use `safeDelete` to validate each target before
deletion.

```js
import { safeDelete } from "pruneguard";

const result = await safeDelete({
  targets: ["src/legacy/old-widget.ts", "src/utils/deprecated.ts"],
});

// Safe targets: delete immediately
for (const entry of result.safe) {
  console.log(`DELETE ${entry.target} (confidence: ${entry.confidence})`);
  // fs.unlinkSync(entry.target);
}

// Needs-review targets: flag for human
for (const entry of result.needsReview) {
  console.warn(`REVIEW ${entry.target}: ${entry.reasons.join("; ")}`);
}

// Blocked targets: do not delete
for (const entry of result.blocked) {
  console.error(`BLOCKED ${entry.target}: ${entry.reasons.join("; ")}`);
}

// Follow the recommended deletion order to avoid breaking intermediate states
console.log("Deletion order:", result.deletionOrder.map(d => d.target));
```

**Key decision points:**

| Classification | Agent action |
|---------------|-------------|
| `safe` with `confidence: "high"` | Delete without human confirmation |
| `safe` with `confidence: "medium"` or `"low"` | Delete but note reduced certainty |
| `needsReview` | Do not delete automatically; flag for human review |
| `blocked` | Never delete; report the blocking reasons |

Always follow `deletionOrder` when deleting multiple files to avoid
breaking import chains in intermediate states.

---

## Agent Decision Logic: fix-plan output

When an agent needs to remediate findings, use `fixPlan` to get a structured
action plan. Each action includes steps that can be executed programmatically.

```js
import { fixPlan } from "pruneguard";

const plan = await fixPlan({
  targets: ["src/legacy/old-widget.ts"],
});

// Check overall risk before proceeding
if (plan.riskLevel === "high") {
  console.warn("High-risk fix plan -- recommend human review before execution.");
}

// Execute actions in order, respecting phases
for (const action of plan.actions) {
  console.log(`[${action.kind}] ${action.targets.join(", ")} (${action.risk} risk, ${action.confidence} confidence)`);

  // Check preconditions
  for (const pre of action.preconditions) {
    console.log(`  Precondition: ${pre}`);
  }

  // Execute steps
  for (const step of action.steps) {
    console.log(`  Step: ${step.description}`);
    if (step.file) console.log(`    File: ${step.file}`);
    if (step.action) console.log(`    Action: ${step.action}`);
  }

  // Verify after execution
  for (const v of action.verification) {
    console.log(`  Verify: ${v}`);
  }
}

// After executing all actions, re-scan to confirm
// const report = await scan({ noCache: true });
// assert(report.findings.length < originalFindingsCount);
```

**Key decision points:**

| Condition | Agent action |
|-----------|-------------|
| `plan.riskLevel === "low"` and `plan.confidence === "high"` | Execute automatically |
| `plan.riskLevel === "medium"` | Execute but verify with re-scan |
| `plan.riskLevel === "high"` | Present plan to human for approval |
| `plan.blockedBy.length > 0` | Resolve blockers first before executing the plan |
| Action `confidence === "low"` | Skip or flag for human review |

Always re-scan with `--no-cache` after executing a fix plan to verify
that the changes resolved the findings without introducing new ones.
