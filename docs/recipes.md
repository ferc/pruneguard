# Recipes

Copy-paste examples using the pruneguard JS API for common automation tasks.

## Scan

Run a full scan and process findings.

```js
import { scan } from "pruneguard";

const report = await scan({
  cwd: "/path/to/repo",
  noCache: true,
  noBaseline: true,
});

console.log(`Total findings: ${report.summary.totalFindings}`);
console.log(`  Errors: ${report.summary.errors}`);
console.log(`  Warnings: ${report.summary.warnings}`);
console.log(`  Info: ${report.summary.infos}`);
console.log(`  Duration: ${report.stats.durationMs}ms`);

// Filter to high-confidence errors only
const critical = report.findings.filter(
  (f) => f.severity === "error" && f.confidence === "high"
);

for (const f of critical) {
  console.log(`${f.code} ${f.subject}: ${f.message}`);
}
```

## Review

Run `review` on a pull request and fail the build if there are blocking
findings. The `review` command classifies findings as blocking (high
confidence + error/warn severity) or advisory.

```js
import { review } from "pruneguard";

const result = await review({
  baseRef: "origin/main",
  noCache: true,
});

console.log("Trust:", JSON.stringify(result.trust));
console.log("Blocking:", result.blockingFindings.length);
console.log("Advisory:", result.advisoryFindings.length);

if (result.blockingFindings.length > 0) {
  console.error("Blocking findings found:");
  for (const f of result.blockingFindings) {
    console.error(`  [${f.confidence}] ${f.code}: ${f.message}`);
  }
  process.exit(1);
}

if (result.recommendations.length > 0) {
  console.log("Recommendations:");
  for (const r of result.recommendations) {
    console.log(`  - ${r}`);
  }
}

console.log("Branch is clean. Safe to merge.");
```

In a GitHub Actions workflow, wrap this in a step:

```yaml
- name: Branch review
  run: node scripts/review-gate.mjs
```

## safeDelete

Before deleting files in an automated cleanup, use `safeDelete` to verify
each target is safe to remove. This prevents breaking changes from automated
scripts.

```js
import { safeDelete } from "pruneguard";

const targets = [
  "src/legacy/old-widget.ts",
  "src/utils/deprecated-helper.ts",
  "src/components/UnusedModal.tsx",
];

const result = await safeDelete({ targets });

// Files safe to delete immediately
if (result.safe.length > 0) {
  console.log("Safe to delete:");
  for (const entry of result.safe) {
    console.log(`  ${entry.target} (${entry.confidence})`);
  }
}

// Files that need human review
if (result.needsReview.length > 0) {
  console.warn("Needs review:");
  for (const entry of result.needsReview) {
    console.warn(`  ${entry.target}: ${entry.reasons.join(", ")}`);
  }
}

// Files that must NOT be deleted
if (result.blocked.length > 0) {
  console.error("Blocked from deletion:");
  for (const entry of result.blocked) {
    console.error(`  ${entry.target}: ${entry.reasons.join(", ")}`);
  }
  process.exit(1);
}

// Delete in the recommended order
console.log("\nRecommended deletion order:");
for (const path of result.deletionOrder) {
  console.log(`  ${path}`);
}
```

## fixPlan

Use `fixPlan` to generate a structured remediation plan with specific actions
per finding, then execute the actions programmatically.

```js
import { fixPlan, scan } from "pruneguard";

// Generate fix plan for specific targets
const plan = await fixPlan({
  targets: ["src/legacy/old-widget.ts"],
});

console.log(`Risk level: ${plan.riskLevel}`);
console.log(`Confidence: ${plan.confidence}`);
console.log(`Actions: ${plan.actions.length}`);

for (const action of plan.actions) {
  console.log(`\n${action.kind}: ${action.targets.join(", ")}`);
  console.log(`  Why: ${action.why}`);
  console.log(`  Risk: ${action.risk}, Confidence: ${action.confidence}`);

  if (action.preconditions.length > 0) {
    console.log("  Preconditions:");
    for (const p of action.preconditions) {
      console.log(`    - ${p}`);
    }
  }

  console.log("  Steps:");
  for (const step of action.steps) {
    console.log(`    - ${step.description}`);
  }
}

if (plan.blockedBy.length > 0) {
  console.warn("\nBlocked by:");
  for (const b of plan.blockedBy) {
    console.warn(`  - ${b}`);
  }
}

console.log("\nVerification steps:");
for (const v of plan.verificationSteps) {
  console.log(`  - ${v}`);
}
```

## run

Use `run` as an escape hatch for CLI flags not covered by the typed API.

```js
import { run } from "pruneguard";

// Run a scan with custom flags
const result = await run([
  "--format", "json",
  "--no-cache",
  "--no-baseline",
  "--daemon", "off",
  "--severity", "error",
  "scan",
]);

console.log(`Exit code: ${result.exitCode}`);
console.log(`Duration: ${result.durationMs}ms`);

if (result.exitCode === 0 || result.exitCode === 1) {
  const report = JSON.parse(result.stdout);
  console.log(`Findings: ${report.findings.length}`);
}
```

## binaryPath

Resolve the native binary for custom integrations.

```js
import { binaryPath, resolutionInfo } from "pruneguard";

// Get just the path
console.log(binaryPath());

// Get full resolution diagnostics
const info = resolutionInfo();
console.log(`Binary: ${info.binaryPath}`);
console.log(`Source: ${info.source}`);
console.log(`Platform: ${info.platform}`);
if (info.platformPackage) {
  console.log(`Package: ${info.platformPackage}`);
}
```

## Baseline-Gated CI

Generate a baseline from the `main` branch scan, then compare subsequent
scans to only surface new findings. This lets teams adopt pruneguard
incrementally without fixing all existing issues first.

**Step 1: Generate baseline on main**

```js
import { scan } from "pruneguard";
import { writeFileSync } from "node:fs";

const report = await scan({
  noCache: true,
  noBaseline: true,
});

writeFileSync("baseline.json", JSON.stringify(report, null, 2));
console.log(`Baseline saved: ${report.findings.length} findings`);
```

**Step 2: Compare on feature branches**

```js
import { scan } from "pruneguard";
import { readFileSync } from "node:fs";

const baseline = JSON.parse(readFileSync("baseline.json", "utf-8"));
const baselineIds = new Set(baseline.findings.map((f) => f.id));

const current = await scan({ noCache: true, noBaseline: true });
const newFindings = current.findings.filter((f) => !baselineIds.has(f.id));

if (newFindings.length > 0) {
  console.error(`${newFindings.length} new finding(s) introduced:`);
  for (const f of newFindings) {
    console.error(`  ${f.id}: ${f.message}`);
  }
  process.exit(1);
}

console.log("No new findings. Branch is clean relative to baseline.");
```

## Fix-Plan Repair Loop

Use the `fix-plan` command output to drive automated fixes.

```js
import { run } from "pruneguard";
import { unlinkSync } from "node:fs";

// Generate fix plan
const result = await run(["--format", "json", "fix-plan"]);
const plan = JSON.parse(result.stdout);

let applied = 0;
let skipped = 0;

for (const action of plan.actions) {
  switch (action.type) {
    case "delete-file":
      console.log(`Deleting: ${action.path}`);
      try {
        unlinkSync(action.path);
        applied++;
      } catch (err) {
        console.warn(`  Failed to delete ${action.path}: ${err.message}`);
        skipped++;
      }
      break;

    case "remove-export":
      console.log(`Remove export: ${action.symbol} from ${action.path}`);
      // Use your preferred AST transform tool here
      skipped++;
      break;

    case "remove-dependency":
      console.log(`Remove dependency: ${action.package} from ${action.manifest}`);
      // Use npm/pnpm to remove the dependency
      skipped++;
      break;

    default:
      console.log(`Unknown action type: ${action.type}`);
      skipped++;
  }
}

console.log(`\nApplied: ${applied}, Skipped: ${skipped}`);

// Re-scan to verify
const verify = await run(["--format", "json", "--no-cache", "--no-baseline", "scan"]);
const report = JSON.parse(verify.stdout);
console.log(`Remaining findings: ${report.findings.length}`);
```

## Impact Analysis Before Merge

Before merging a PR that modifies shared code, run `impact` on each changed
file to understand the blast radius.

```js
import { impact } from "pruneguard";
import { execSync } from "node:child_process";

// Get changed files from git
const diffOutput = execSync("git diff --name-only origin/main...HEAD", {
  encoding: "utf-8",
});
const changedFiles = diffOutput
  .trim()
  .split("\n")
  .filter((f) => /\.(ts|tsx|js|jsx|mts|mjs)$/.test(f));

console.log(`Analyzing impact of ${changedFiles.length} changed file(s):\n`);

let totalAffected = new Set();

for (const file of changedFiles) {
  try {
    const result = await impact({ target: file });
    const affected = result.affectedEntrypoints.length + result.affectedFiles.length;

    if (affected > 0) {
      console.log(`${file}:`);
      console.log(`  Entrypoints affected: ${result.affectedEntrypoints.length}`);
      console.log(`  Files affected: ${result.affectedFiles.length}`);
      console.log(`  Packages affected: ${result.affectedPackages.join(", ") || "none"}`);
      console.log();

      for (const f of result.affectedFiles) totalAffected.add(f);
      for (const e of result.affectedEntrypoints) totalAffected.add(e);
    }
  } catch (err) {
    // File may be new or deleted -- impact may not resolve
    console.warn(`  Skipped ${file}: ${err.message}`);
  }
}

console.log(`\nTotal blast radius: ${totalAffected.size} unique affected nodes`);

if (totalAffected.size > 50) {
  console.warn(
    "WARNING: Large blast radius. Consider splitting this PR or " +
    "requesting additional reviewers."
  );
}
```

## Suggest Rules and Apply

Discover governance rules that match your repo's actual dependency patterns.

```js
import { suggestRules, loadConfig } from "pruneguard";
import { writeFileSync, readFileSync } from "node:fs";

const result = await suggestRules();

console.log(`Suggested ${result.suggestedRules.length} rule(s):`);
for (const rule of result.suggestedRules) {
  console.log(`  [${rule.confidence}] ${rule.name}: ${rule.description}`);
}

if (result.hotspots?.length) {
  console.log("\nHotspots:");
  for (const h of result.hotspots) {
    console.log(`  ${h.file}: ${h.suggestion}`);
  }
}

// Merge suggested rules into existing config
const config = JSON.parse(readFileSync("pruneguard.json", "utf-8"));
config.rules = config.rules || {};
config.rules.forbidden = config.rules.forbidden || [];

for (const rule of result.suggestedRules.filter((r) => r.confidence === "high")) {
  config.rules.forbidden.push(rule.configFragment);
}

writeFileSync("pruneguard.json", JSON.stringify(config, null, 2));
console.log("Updated pruneguard.json with high-confidence rules.");
```
