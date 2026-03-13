# pruneguard

Repo truth engine for JS/TS monorepos.

Build one accurate repo graph, then answer many high-value questions cheaply:
unused exports, unused files, unused dependencies, cycles, boundary violations,
ownership visibility, blast-radius analysis, and CI/agent-safe refactor checks.

## Why pruneguard

- **Rust native binary** -- fast full-repo graph builds
- Unused exports, files, and dependencies detection
- Cycle detection and boundary rules
- Confidence scoring (high / medium / low)
- Built-in branch review gate (`review`)
- Safe-delete evaluation and fix-plan remediation
- Blast-radius analysis (`impact`) and proof chains (`explain`)
- SARIF output and deterministic ordering

## Quick start

```sh
# Install (auto-selects the correct native binary for your platform)
npm install pruneguard

# Review your branch (the main command for daily use)
npx pruneguard review

# Review with change detection
npx pruneguard --changed-since origin/main review

# Full repository scan
npx pruneguard scan

# Check if files are safe to delete
npx pruneguard safe-delete src/legacy/old-widget.ts

# Get a remediation plan
npx pruneguard fix-plan src/legacy/old-widget.ts

# Generate a config file with editor autocomplete
npx pruneguard init
```

No Rust toolchain, no compilation, no native addons -- just `npm install`
and go. Requires Node.js >= 18. Supported: macOS (ARM64, x64), Linux
(x64/ARM64, glibc and musl), Windows (x64, ARM64).

See [docs/getting-started.md](docs/getting-started.md) for a full
install-to-first-result walkthrough.

## Daily workflow

`pruneguard review` is the one command most developers and CI systems should use:
- Full-scope analysis with trust summary
- Blocking vs advisory finding split
- Machine-readable proposed actions for AI agents
- Exit 0 (no blockers) or exit 1 (blockers present)

## How it works

pruneguard ships a compiled Rust binary for each supported platform. The JS API
and CLI both spawn this binary behind the scenes. On your local machine the
daemon keeps the graph warm in memory for sub-millisecond queries. In CI (or
when you pass `--daemon off`) every invocation is a fresh one-shot run.

```
npm install pruneguard
       |
       v
@pruneguard/cli-<platform>  <-- native binary, auto-selected by OS+arch
       |
       v
pruneguard (JS wrapper)     <-- spawns the binary, parses JSON output
       |
       +-- CLI: npx pruneguard scan
       +-- JS API: import { scan } from "pruneguard"
```

**Default execution mode:**

| Context         | Mode    | Why                                      |
|-----------------|---------|------------------------------------------|
| Local terminal  | daemon  | Warm graph, instant `review` and `impact`|
| CI / `--daemon off` | one-shot | Deterministic, no lingering process  |

## CLI

### Commands

```sh
# Analysis
pruneguard scan [paths...]               # Full or partial-scope scan
pruneguard impact <target>               # Blast-radius analysis
pruneguard explain <query>               # Proof chain for a finding or path
pruneguard review                        # Branch review gate (blocking vs advisory)
pruneguard safe-delete <targets...>      # Evaluate targets for safe deletion
pruneguard fix-plan <targets...>         # Structured remediation plan
pruneguard suggest-rules                 # Auto-suggest governance rules

# Configuration
pruneguard init                          # Generate pruneguard.json
pruneguard print-config                  # Print resolved config

# Debugging
pruneguard debug resolve <spec> --from <file>  # Trace module resolution
pruneguard debug entrypoints                    # List detected entrypoints
pruneguard debug runtime                        # Print binary/platform info

# Daemon
pruneguard daemon start|stop|status      # Manage the background daemon

```

### Global flags

```
-c, --config <FILE>          Config file path [default: pruneguard.json]
    --format <FORMAT>        text | json | sarif | dot
    --profile <PROFILE>      production | development | all
    --changed-since <REF>    Only report findings for changed files
    --focus <GLOB>           Filter findings to matching paths
    --severity <SEVERITY>    Minimum severity: error | warn | info
    --no-cache               Disable incremental cache
    --no-baseline            Disable baseline suppression
    --max-findings <N>       Cap reported findings
    --require-full-scope     Fail if scan would be partial-scope
    --daemon <MODE>          auto | off | required
```

### Common CLI workflows

```sh
# Full scan
pruneguard scan

# Focus to a slice of the repo (full analysis, filtered output)
pruneguard --focus "src/**" scan

# Changed-since review for CI/agents
pruneguard --changed-since origin/main scan

# Deterministic CI without baseline influence
pruneguard --no-baseline --no-cache scan

# Fail advisory dead-code scans in automation
pruneguard --require-full-scope scan

# Partial-scope scan (dead-code findings are advisory)
pruneguard scan src/components/Button.tsx src/lib/utils.ts

# Blast radius
pruneguard impact src/utils/helpers.ts

# Explain a finding
pruneguard explain unused-export:packages/core:src/old.ts#deprecatedFn

# Branch review (CI/agent gate)
pruneguard --changed-since origin/main review

# Safe-delete check
pruneguard safe-delete src/utils/old-helper.ts src/legacy/widget.ts

# JSON for CI
pruneguard --no-baseline --no-cache --format json scan

# SARIF for GitHub Code Scanning
pruneguard --format sarif scan > results.sarif

# Graphviz DOT output
pruneguard --format dot scan | dot -Tsvg -o graph.svg

# Generate config
pruneguard init

# Debug resolution
pruneguard debug resolve ./utils --from src/index.ts

# Debug runtime/install info
pruneguard debug runtime
```

## JS API

Every function spawns the native binary and returns parsed, typed results.
See [docs/js-api.md](docs/js-api.md) for the complete API reference.

### scan

```js
import { scan } from "pruneguard";

const report = await scan({
  cwd: "/path/to/repo",       // optional, defaults to process.cwd()
  profile: "production",       // optional: "production" | "development" | "all"
  changedSince: "origin/main", // optional
  focus: "packages/core/**",   // optional
  noCache: true,               // optional
  noBaseline: true,            // optional
  requireFullScope: true,      // optional
  paths: ["src/lib"],          // optional, partial-scope scan
});

console.log(report.summary.totalFindings);
console.log(report.findings[0].id, report.findings[0].confidence);
```

### review

```js
import { review } from "pruneguard";

const result = await review({
  baseRef: "origin/main",
  noCache: true,
});

console.log("Blocking:", result.blockingFindings.length);
console.log("Advisory:", result.advisoryFindings.length);
console.log("Trust:", JSON.stringify(result.trust));

if (result.blockingFindings.length > 0) {
  for (const f of result.blockingFindings) {
    console.error(`  [${f.confidence}] ${f.code}: ${f.message}`);
  }
  process.exit(1);
}
```

### safeDelete

```js
import { safeDelete } from "pruneguard";

const result = await safeDelete({
  targets: ["src/legacy/old-widget.ts", "src/utils/deprecated-helper.ts"],
});

console.log("Safe:", result.safe.map(e => e.target));
console.log("Blocked:", result.blocked.map(e => `${e.target}: ${e.reasons.join(", ")}`));
console.log("Deletion order:", result.deletionOrder);
```

### fixPlan

```js
import { fixPlan } from "pruneguard";

const plan = await fixPlan({
  targets: ["unused-export:packages/core:src/old.ts#deprecatedFn"],
});

for (const action of plan.actions) {
  console.log(`${action.kind}: ${action.targets.join(", ")} (${action.risk} risk)`);
  for (const step of action.steps) {
    console.log(`  - ${step.description}`);
  }
}
```

### run

```js
import { run } from "pruneguard";

// Run arbitrary CLI args
const result = await run(["--format", "json", "--no-cache", "scan"]);
console.log(result.exitCode);
console.log(result.stdout);
console.log(result.durationMs);
```

### binaryPath

```js
import { binaryPath } from "pruneguard";

// Resolve the native binary path (for custom integrations)
console.log(binaryPath());
// => /path/to/node_modules/@pruneguard/cli-darwin-arm64/bin/pruneguard
```

### Other API functions

```js
import {
  impact,
  explain,
  suggestRules,
  loadConfig,
  schemaPath,
  scanDot,
  resolutionInfo,
  debugResolve,
  debugEntrypoints,
} from "pruneguard";

// Blast radius
const blast = await impact({ target: "src/utils/helpers.ts" });
console.log(blast.affectedEntrypoints, blast.affectedFiles);

// Proof chain
const proof = await explain({ query: "src/old.ts#deprecatedFn" });
console.log(proof.proofs);

// Suggest governance rules from graph analysis
const rules = await suggestRules();
console.log(rules.suggestedRules);

// Load resolved config
const config = await loadConfig();

// Path to the bundled JSON schema
console.log(schemaPath());

// Graphviz DOT output
const dot = await scanDot();

// Binary resolution diagnostics
const info = resolutionInfo();
console.log(info.source, info.platform);
```

### Full API reference

| Function | Signature | Description |
|---|---|---|
| `scan` | `(options?) => Promise<AnalysisReport>` | Full or partial-scope repo scan |
| `impact` | `(options) => Promise<ImpactReport>` | Blast-radius analysis for a target |
| `explain` | `(options) => Promise<ExplainReport>` | Proof chain for a finding or path |
| `review` | `(options?) => Promise<ReviewReport>` | Branch review gate |
| `safeDelete` | `(options) => Promise<SafeDeleteReport>` | Evaluate targets for safe deletion |
| `fixPlan` | `(options) => Promise<FixPlanReport>` | Structured remediation plan |
| `suggestRules` | `(options?) => Promise<SuggestRulesReport>` | Auto-suggest governance rules |
| `loadConfig` | `(options?) => Promise<PruneguardConfig>` | Load resolved config |
| `schemaPath` | `() => string` | Path to bundled configuration JSON schema |
| `binaryPath` | `() => string` | Path to the resolved native binary |
| `run` | `(args, options?) => Promise<CommandResult>` | Run arbitrary CLI args |
| `scanDot` | `(options?) => Promise<string>` | Graphviz DOT output |

### Error handling

All API functions throw `PruneguardExecutionError` on failure. The error
carries a `code` field for programmatic handling:

| Code | Meaning |
|---|---|
| `PRUNEGUARD_BINARY_NOT_FOUND` | Native binary could not be located |
| `PRUNEGUARD_EXECUTION_FAILED` | Binary exited with an unexpected code |
| `PRUNEGUARD_JSON_PARSE_FAILED` | Binary output was not valid JSON |

```js
import { scan, PruneguardExecutionError } from "pruneguard";

try {
  await scan();
} catch (err) {
  if (err instanceof PruneguardExecutionError) {
    console.error(err.code, err.message);
    console.error("stderr:", err.stderr);
  }
}
```

## GitHub Actions

pruneguard includes a reusable [GitHub Action](.github/actions/pruneguard/)
for CI integration. See [docs/ci-integration.md](docs/ci-integration.md) for
complete setup guides including baseline workflows, SARIF, and monorepo
strategies.

### Branch review gate

```yaml
name: pruneguard
on: [pull_request]

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Branch review
        run: npx pruneguard --changed-since origin/main --format json review
```

Exit code 0 means no blocking findings; exit code 1 means blocking findings
exist. The JSON output contains `blockingFindings` and `advisoryFindings`
arrays for further processing.

### Baseline-gated CI

Adopt pruneguard incrementally by saving a baseline on `main` and only
failing on new findings.

```yaml
name: pruneguard-baseline
on:
  push:
    branches: [main]
  pull_request:

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      # On main: save baseline
      - name: Save baseline
        if: github.ref == 'refs/heads/main'
        run: npx pruneguard --no-cache --no-baseline --format json scan > baseline.json

      - uses: actions/upload-artifact@v6
        if: github.ref == 'refs/heads/main'
        with:
          name: pruneguard-baseline
          path: baseline.json

      # On PRs: compare against baseline
      - uses: actions/download-artifact@v7
        if: github.event_name == 'pull_request'
        with:
          name: pruneguard-baseline
        continue-on-error: true

      - name: Check for new findings
        if: github.event_name == 'pull_request'
        run: |
          npx pruneguard --no-cache --no-baseline --format json scan > current.json
          node -e "
            const fs = require('fs');
            if (!fs.existsSync('baseline.json')) { console.log('No baseline found, skipping comparison'); process.exit(0); }
            const baseline = JSON.parse(fs.readFileSync('baseline.json', 'utf-8'));
            const current = JSON.parse(fs.readFileSync('current.json', 'utf-8'));
            const baseIds = new Set(baseline.findings.map(f => f.id));
            const newFindings = current.findings.filter(f => !baseIds.has(f.id));
            if (newFindings.length > 0) {
              console.error(newFindings.length + ' new finding(s):');
              newFindings.forEach(f => console.error('  ' + f.id + ': ' + f.message));
              process.exit(1);
            }
            console.log('No new findings relative to baseline.');
          "
```

### Safe-delete review

Verify that candidates marked for removal are actually safe to delete
before an automated cleanup PR merges.

```yaml
name: safe-delete-check
on:
  pull_request:
    paths:
      - "scripts/cleanup-*.mjs"

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Check deletion safety
        run: |
          # Find files deleted in this PR
          DELETED=$(git diff --name-only --diff-filter=D origin/main...HEAD | grep -E '\.(ts|tsx|js|jsx|mts|mjs)$' || true)
          if [ -z "$DELETED" ]; then
            echo "No source files deleted in this PR."
            exit 0
          fi
          echo "Checking deletion safety for:"
          echo "$DELETED"
          npx pruneguard --format json safe-delete $DELETED
```

## Trust model

- Full-repo `scan` is the trustworthy mode for deletion decisions.
- `--focus` filters reported findings after full analysis.
- Positional `scan <paths...>` narrows the analyzed file set and is reported
  as partial-scope/advisory.
- `--require-full-scope` turns advisory partial-scope dead-code scans into a
  hard failure (exit 2).
- `--no-baseline` disables baseline auto-discovery for deterministic CI,
  parity, and benchmarks.
- Use `impact` and `explain` before removing code on repos with many
  unresolved specifiers.
- Findings carry `confidence` (high, medium, low) to indicate
  trustworthiness.

## Configuration

Create `pruneguard.json` (or `.pruneguardrc.json`). Run `pruneguard init` to
generate a starter config.

```json
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json",
  "workspaces": {
    "packageManager": "pnpm",
    "roots": ["apps/*", "packages/*"]
  },
  "analysis": {
    "unusedExports": "error",
    "unusedFiles": "warn",
    "unusedDependencies": "error",
    "cycles": "warn"
  }
}
```

See [docs/config.md](docs/config.md) for the full configuration reference.

## Documentation

| Guide | Description |
|---|---|
| [Getting started](docs/getting-started.md) | Install-to-first-result walkthrough |
| [Configuration](docs/config.md) | Full configuration reference |
| [CI integration](docs/ci-integration.md) | GitHub Actions, baseline workflows, SARIF |
| [JS API reference](docs/js-api.md) | Complete typed API documentation |
| [Agent integration](docs/agent-integration.md) | AI agent workflows with review + safe-delete + fix-plan |
| [Recipes](docs/recipes.md) | Copy-paste automation examples |
| [Migration](docs/migration.md) | Migrate from other tools |
| [Architecture](docs/architecture.md) | Internal design and pipeline stages |
| [Performance](docs/performance.md) | Performance model, cache behavior, benchmarking |
| [Benchmarks](docs/benchmarks.md) | Target latencies and benchmark methodology |

## Development

Requires: Rust (stable), Node.js, pnpm, just

```sh
just build-js                    # Build the JS wrapper
just stage-release               # Stage npm packages into .release/
just pack-smoke                  # End-to-end package install smoke test
just smoke-repos                 # Opt-in real-repo smoke tests
just parity                      # Real-repo parity checks
just benchmark CASE=../../repo   # Benchmark a single corpus
just benchmark-repos             # Benchmark all configured corpora
```

Other useful commands:

```sh
just ready          # fmt + check + test + lint
just build          # Release binary
just run scan       # Run against current directory
just schemas        # Regenerate shipped schemas
just schemas-check  # Verify schemas are committed
just ci             # Full CI pipeline locally
```

## License

MIT
