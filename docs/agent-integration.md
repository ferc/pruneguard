# Agent Integration

This document describes how AI coding agents (Claude, Cursor, Copilot,
custom bots) can use pruneguard to make safe, evidence-based code changes.

pruneguard is designed for machine consumption: every command produces
structured JSON output, findings carry confidence scores, and remediation
plans contain step-by-step instructions that agents can execute directly.

## Why agents need pruneguard

Agents editing large codebases face three problems:

1. **Is this code actually unused?** An agent cannot reliably grep a 5000-file
   monorepo for all possible import patterns. pruneguard builds a full module
   graph and answers this with confidence levels.

2. **Is it safe to delete this file?** Even if something looks unused, it
   might be imported dynamically, referenced from a framework config, or used
   in a script. `safe-delete` evaluates deletion safety with evidence.

3. **What else breaks if I change this?** `impact` shows the blast radius of
   a change before the agent makes it.

## Install

```sh
npm install pruneguard
```

No Rust toolchain required. The package ships a pre-built native binary.

## Core workflow: review, then act

The recommended agent workflow has three phases:

### Phase 1: Review the branch

```js
import { review } from "pruneguard";

const result = await review({
  baseRef: "origin/main",
  noCache: true,
});

// Check if the branch introduces problems
if (result.blockingFindings.length > 0) {
  // Agent should address these before the PR can merge
  for (const f of result.blockingFindings) {
    console.log(`${f.code} ${f.subject}: ${f.message}`);
    console.log(`  Confidence: ${f.confidence}`);
    console.log(`  Suggestion: ${f.suggestion}`);
  }
}

// Advisory findings are informational -- agent can mention them but
// should not block on them
for (const f of result.advisoryFindings) {
  console.log(`Advisory: ${f.code} ${f.subject}`);
}

// Trust summary tells the agent how much to trust the results
console.log("Full scope:", result.trust.fullScope);
console.log("Unresolved pressure:", result.trust.unresolvedPressure);
```

### Phase 2: Plan the fix

```js
import { fixPlan } from "pruneguard";

// Get a structured plan for the blocking findings
const plan = await fixPlan({
  targets: result.blockingFindings.map(f => f.subject),
});

for (const action of plan.actions) {
  // Each action is a discrete, executable step
  console.log(`Action: ${action.kind}`);
  console.log(`Targets: ${action.targets.join(", ")}`);
  console.log(`Risk: ${action.risk}`);

  for (const step of action.steps) {
    console.log(`  Step: ${step.description}`);
    if (step.file) console.log(`  File: ${step.file}`);
    if (step.action) console.log(`  Action: ${step.action}`);
  }
}
```

### Phase 3: Verify the fix

```js
import { scan } from "pruneguard";

// After making changes, re-scan to verify
const report = await scan({ noCache: true });

const remaining = report.findings.filter(f =>
  f.severity === "error" && f.confidence === "high"
);

if (remaining.length === 0) {
  console.log("All blocking findings resolved.");
}
```

## Safe deletion workflow

When an agent needs to remove files (dead code cleanup, refactoring):

```js
import { safeDelete, impact } from "pruneguard";

const targets = [
  "src/legacy/old-widget.ts",
  "src/utils/deprecated-helper.ts",
];

// Step 1: Check blast radius before deletion
for (const target of targets) {
  const blast = await impact({ target });
  console.log(`${target}: affects ${blast.affectedFiles.length} files`);
}

// Step 2: Verify deletion safety
const result = await safeDelete({ targets });

// Only delete files classified as safe
const safeTargets = result.safe.map(e => e.target);
const blockedTargets = result.blocked.map(e => e.target);

// Step 3: Delete in the recommended order
for (const path of result.deletionOrder) {
  if (safeTargets.includes(path)) {
    // Agent can safely delete this file
    console.log(`Deleting: ${path}`);
  }
}

// Step 4: Report blocked targets
if (blockedTargets.length > 0) {
  console.log("Cannot delete (still in use):");
  for (const entry of result.blocked) {
    console.log(`  ${entry.target}: ${entry.reasons.join(", ")}`);
  }
}
```

## Fix-plan repair loop

For automated cleanup tasks, the agent can run a repair loop:

```js
import { fixPlan, scan } from "pruneguard";

// Initial scan to find issues
let report = await scan({ noCache: true, noBaseline: true });
let errors = report.findings.filter(f => f.severity === "error");

while (errors.length > 0) {
  const plan = await fixPlan({
    targets: errors.slice(0, 10).map(f => f.subject),
  });

  if (plan.actions.length === 0) break;

  for (const action of plan.actions) {
    if (action.risk === "high") {
      console.log(`Skipping high-risk action: ${action.kind}`);
      continue;
    }

    // Execute the action steps
    for (const step of action.steps) {
      // Agent applies the step (delete file, remove export, etc.)
      console.log(`Applying: ${step.description}`);
    }
  }

  // Re-scan to verify
  report = await scan({ noCache: true, noBaseline: true });
  errors = report.findings.filter(f => f.severity === "error");
}
```

## Using trust scores

Every pruneguard finding carries a confidence level. Agents should use this
to decide how to act:

| Confidence | Agent behavior |
|------------|---------------|
| `high`     | Act automatically. The finding is strongly supported by graph evidence. |
| `medium`   | Act with caution. Consider running `explain` for more context. |
| `low`      | Flag for human review. Do not auto-delete or auto-fix. |

The `review` command also returns a `trust` object with aggregate metrics:

```js
const result = await review({ baseRef: "origin/main" });

if (result.trust.unresolvedPressure > 0.1) {
  // More than 10% of specifiers are unresolved -- results are less reliable
  console.log("High unresolved pressure; agent should be conservative.");
}

if (!result.trust.fullScope) {
  // The scan was partial-scope; dead-code findings are advisory only
  console.log("Partial scope; dead-code findings are advisory.");
}
```

## CLI usage for agents

Agents that prefer to shell out can use the CLI directly:

```sh
# Branch review
pruneguard --format json --changed-since origin/main review

# Safe-delete check
pruneguard --format json safe-delete src/old.ts src/legacy/widget.ts

# Fix plan
pruneguard --format json fix-plan src/old.ts

# Blast radius
pruneguard --format json impact src/utils/helpers.ts

# Proof chain
pruneguard --format json explain src/old.ts#deprecatedFn
```

All commands exit 0 when clean and 1 when findings/blockers exist. The JSON
output is fully typed and deterministically ordered.

## Providing pruneguard context to agents

If you are configuring an AI agent's system prompt or tool definitions, here
is a summary of each command's purpose:

| Command       | Purpose | When to use |
|---------------|---------|-------------|
| `review`      | Branch gate: blocking vs advisory findings with trust metadata | Before approving a PR or deciding what to fix |
| `safe-delete` | Deletion safety check: safe / needs-review / blocked | Before removing any file |
| `fix-plan`    | Structured remediation: actions, steps, risk levels | When the agent needs to fix findings |
| `impact`      | Blast-radius analysis: affected files, packages, entrypoints | Before editing shared code |
| `explain`     | Proof chain: why something is unused or violating a rule | When a finding is unclear |
| `scan`        | Full analysis: all findings with inventories and stats | For initial assessment of a codebase |

## MCP / tool-use integration

For agents using the Model Context Protocol (MCP) or similar tool-use
frameworks, each pruneguard JS function maps directly to a tool:

```json
{
  "name": "pruneguard_review",
  "description": "Run a branch review to find blocking and advisory findings",
  "parameters": {
    "baseRef": { "type": "string", "description": "Git ref to compare against (e.g. origin/main)" },
    "noCache": { "type": "boolean", "description": "Disable incremental cache" }
  }
}
```

The JS API functions (`review`, `safeDelete`, `fixPlan`, `impact`, `explain`,
`scan`) accept typed option objects and return typed results, making them
straightforward to wrap as tools.

## Example: Claude Code integration

An agent using Claude Code can invoke pruneguard via the JS API or CLI:

```js
// In a tool definition or script the agent can call:
import { review, safeDelete, fixPlan } from "pruneguard";

// The agent's workflow:
// 1. Run review to understand the state of the branch
// 2. Use fixPlan to get remediation steps for blocking findings
// 3. Apply the steps
// 4. Use safeDelete before removing any files
// 5. Re-run review to verify the branch is clean
```

## Best practices for agent integration

1. **Always check confidence.** Do not auto-delete files based on low-
   confidence findings.

2. **Use `safe-delete` before every deletion.** Even if `scan` says a file
   is unused, `safe-delete` performs additional checks (dynamic imports,
   framework references, script usage).

3. **Respect `blocked` results.** If `safe-delete` says a file is blocked,
   the agent must not delete it. Show the reasons to the user.

4. **Check trust metrics.** If `unresolvedPressure` is high, the agent
   should be more conservative and flag findings for human review.

5. **Use `--no-cache` in agent loops.** When making changes and re-scanning,
   always pass `noCache: true` to avoid stale results.

6. **Follow `deletionOrder`.** When deleting multiple files, use the order
   returned by `safe-delete` to avoid temporarily breaking imports.

7. **Limit repair loop iterations.** Set a maximum number of fix-plan
   iterations to avoid infinite loops when findings cannot be resolved.
