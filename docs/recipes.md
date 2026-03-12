# Recipes

Copy-paste examples using the pruneguard JS API for common automation tasks.

## Branch Review in CI

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
  env:
    NODE_OPTIONS: "--experimental-vm-modules"
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

## Safe Deletion Checks

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

## Fix-Plan Repair Loop

Use the `fix-plan` command output to drive automated fixes. The fix-plan
produces a structured remediation plan with specific actions per finding.

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
file to understand the blast radius. This helps reviewers focus on the files
most likely to be affected.

```js
import { impact, scan } from "pruneguard";
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
    // File may be new or deleted — impact may not resolve
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
