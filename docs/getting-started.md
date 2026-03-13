# Getting Started

This guide takes you from zero to your first pruneguard results in under five
minutes. No Rust toolchain, no compilation -- just `npm install` and go.

## Prerequisites

- Node.js >= 18
- A JS/TS project (single package or monorepo)

## Install

```sh
npm install pruneguard
```

The `pruneguard` package automatically installs the correct native binary for
your platform. Supported: macOS (ARM64, x64), Linux (x64/ARM64, glibc and
musl), Windows (x64, ARM64).

## Review your branch

The bare `pruneguard` command is all you need. No config file required. It
runs a review of your current branch, classifying findings as **blocking**
(high-confidence errors/warnings) or **advisory**. This is the primary
command for CI and agent workflows:

```sh
npx pruneguard
```

By default this detects your base branch automatically. To be explicit:

```sh
npx pruneguard --changed-since origin/main
```

Exit code 0 means safe to merge. Exit code 1 means blocking findings exist.

Example output:

```
BLOCKING:
  error [high] unused-export packages/core/src/old.ts#deprecatedFn
    Not imported by any reachable module.

ADVISORY:
  warn [medium] unused-file packages/legacy/src/widget.ts
    No incoming imports from any entrypoint.

Trust: fullScope=true, unresolvedPressure=0.01, baseline=applied
1 blocking, 1 advisory, 342 files, 14ms
```

For JSON output (recommended for CI and agents):

```sh
npx pruneguard --format json
```

## Full-repo scan

Run a full-repo scan from your project root to see everything:

```sh
npx pruneguard scan
```

pruneguard builds a complete module graph of your repository and reports
unused files, unused exports, unused dependencies, cycles, and boundary
violations. The text output shows each finding with its severity, confidence
level, and evidence.

Example output:

```
error [high] unused-export packages/core/src/old.ts#deprecatedFn
  Not imported by any reachable module.

warn [medium] unused-file packages/legacy/src/widget.ts
  No incoming imports from any entrypoint.

2 findings (1 error, 1 warning), 342 files, 12ms
```

## Generate a config

Most repos work without a config file. If you need custom rules, ownership,
or framework overrides, run `pruneguard init` to generate a minimal config:

```sh
npx pruneguard init
```

This creates a `pruneguard.json` with just the schema reference:

```json
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json"
}
```

See [config.md](config.md) for the full configuration reference.

## Focus on what matters

### Filter by path

Show only findings in a specific area of the repo:

```sh
npx pruneguard --focus "packages/core/**" scan
```

The full graph is still built and analyzed; `--focus` only filters the
reported findings.

### Filter by severity

Only show errors:

```sh
npx pruneguard --severity error scan
```

### Changed-since review

Only report findings related to files changed since a base branch:

```sh
npx pruneguard --changed-since origin/main
```

## Branch review gate

The bare `pruneguard` command is built for CI and agent workflows. It
classifies findings as blocking (high-confidence errors/warnings) or
advisory, and exits 0 when there are no blockers:

```sh
npx pruneguard
```

Exit code 0 = safe to merge. Exit code 1 = blocking findings exist.

## Check before you delete

Before removing files from your repo, verify they are safe to delete:

```sh
npx pruneguard safe-delete src/legacy/old-widget.ts src/utils/deprecated.ts
```

pruneguard classifies each target as safe, needs-review, or blocked, with
reasons and a recommended deletion order.

## Get a fix plan

Generate a structured remediation plan for specific findings or files:

```sh
npx pruneguard fix-plan src/legacy/old-widget.ts
```

The plan includes specific actions, steps, risk levels, and verification
instructions.

## JSON output

Every command supports `--format json` for machine-readable output:

```sh
npx pruneguard --format json
npx pruneguard --format json scan
npx pruneguard --format json safe-delete src/old.ts
```

## Understand blast radius

Before editing shared code, check what would be affected:

```sh
npx pruneguard impact src/utils/helpers.ts
```

## Explain a finding

Get a proof chain for why something is unused or violating a boundary:

```sh
npx pruneguard explain src/old.ts#deprecatedFn
```

## Use the JS API

All CLI commands are available as typed JS functions:

```js
import { scan, review, safeDelete, fixPlan } from "pruneguard";

const report = await scan();
console.log(report.summary.totalFindings);
```

See [js-api.md](js-api.md) for the full API reference.

## Set up CI

Add pruneguard to your CI pipeline with the GitHub Action or raw CLI
commands. See [ci-integration.md](ci-integration.md) for complete examples.

## Migrate from knip or dependency-cruiser

If you already use knip or dependency-cruiser, pruneguard can convert your
existing config:

```sh
npx pruneguard migrate knip
npx pruneguard migrate depcruise
```

See [migration.md](migration.md) for details.

## Next steps

- [Configuration reference](config.md) -- all config options
- [CI integration](ci-integration.md) -- GitHub Actions, baseline workflows
- [JS API reference](js-api.md) -- programmatic usage
- [Agent usage](agent-usage.md) -- AI agent workflows and decision logic
- [Competitive positioning](competitive.md) -- vs knip, vs dependency-cruiser
- [Recipes](recipes.md) -- copy-paste automation examples
