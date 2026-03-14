# pruneguard

Find unused exports, dead files, phantom dependencies, import cycles, and
boundary violations across your entire JS/TS monorepo -- in a single, fast
Rust-powered pass.

Built on [oxc](https://oxc.rs), pruneguard scans a 5,000+ file repository in
under 6 seconds cold, and delivers sub-millisecond responses with the warm
daemon. That is 10-50x faster than JS-based alternatives -- fast enough for an
AI agent to call on every single code change without slowing anything down.

No Rust toolchain, no compilation, no native addons -- just `npm install` and
go.

## Use with AI agents

pruneguard was designed for AI agents: every command produces structured JSON,
findings carry confidence scores, and remediation plans contain step-by-step
instructions agents can execute directly. Because it runs in milliseconds, your
agent can call it after every code change -- something that is impractical with
slower JS-based tools.

### Prompt: find and remove dead code

Copy-paste this into your agent configuration (`CLAUDE.md`, `.cursorrules`,
Copilot instructions, etc.):

> When asked to clean up dead code, or after making significant refactoring
> changes, run `npx pruneguard --format json scan` to detect unused exports,
> dead files, and phantom dependencies. For each finding with `"high"`
> confidence, run `npx pruneguard --format json safe-delete <target>` to
> confirm it is safe to remove. Only delete targets that pruneguard marks as
> `"safe"`. Follow the `deletionOrder` array when removing multiple files.
> After all deletions, re-run the scan with `--no-cache` to verify no new
> issues were introduced. Do not delete anything with `"low"` confidence
> without asking the user first.

### Prompt: set up pruneguard and guard against new dead code

> Add pruneguard as a dev dependency and configure it to catch dead code on
> every change:
>
> 1. Run `npm install -D pruneguard` (or the equivalent for the project's
>    package manager).
> 2. Add these scripts to `package.json`:
>    ```json
>    "review": "pruneguard --changed-since origin/main",
>    "scan": "pruneguard scan",
>    "prune:check": "pruneguard --changed-since origin/main --format json"
>    ```
> 3. Run `npx pruneguard init` to generate `pruneguard.json` with schema
>    reference.
> 4. If the project uses CI (GitHub Actions), add a step that runs
>    `npx pruneguard --changed-since origin/main --format json` on pull
>    requests to block PRs that introduce new dead code.
>
> pruneguard is a compiled Rust binary -- scanning takes milliseconds with the
> warm daemon, so it adds no meaningful overhead to the development workflow or
> CI pipeline.

See [docs/agent-integration.md](docs/agent-integration.md) for the full agent
integration guide, including JS API workflows and MCP tool definitions.

## Quick start

```sh
npx pruneguard            # zero-install, scan your repo right now
npm install -D pruneguard # or add it as a dev dependency
```

Add scripts to your `package.json`:

```json
{
  "scripts": {
    "review": "pruneguard",
    "scan": "pruneguard scan",
    "prune:delete": "pruneguard safe-delete"
  }
}
```

Run it:

```sh
# Review your repo (the default command)
npx pruneguard

# Review only what changed on your branch
npx pruneguard --changed-since origin/main

# Full detailed scan
npx pruneguard scan

# Check if a file is safe to delete
npx pruneguard safe-delete src/legacy/old-widget.ts

# Get a remediation plan
npx pruneguard fix-plan src/legacy/old-widget.ts
```

Requires Node.js >= 18. Supported platforms: macOS (ARM64, x64), Linux
(x64/ARM64, glibc and musl), Windows (x64, ARM64).

See [docs/getting-started.md](docs/getting-started.md) for a full
install-to-first-result walkthrough.

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
       +-- CLI: npx pruneguard
       +-- JS API: import { review } from "pruneguard"
```

| Context             | Mode     | Why                                       |
|---------------------|----------|-------------------------------------------|
| Local terminal      | daemon   | Warm graph, instant `review` and `impact` |
| CI / `--daemon off` | one-shot | Deterministic, no lingering process       |

## CLI

### Commands

#### Daily use

```sh
pruneguard                          # Review your repo or branch (default command)
pruneguard scan [paths...]          # Full repo scan with detailed findings
pruneguard safe-delete <targets...> # Check if files or exports are safe to remove
pruneguard fix-plan <targets...>    # Generate a remediation plan
```

#### Investigation

```sh
pruneguard impact <target>          # Analyze blast radius for a target
pruneguard explain <query>          # Explain a finding with proof chain
```

#### Policy and governance

```sh
pruneguard suggest-rules            # Auto-suggest governance rules from graph analysis
```

#### Setup

```sh
pruneguard init                     # Generate pruneguard.json with schema reference
pruneguard print-config             # Print resolved config
```

#### Debugging and migration

```sh
pruneguard debug resolve <spec> --from <file>  # Trace module resolution
pruneguard debug entrypoints                    # List detected entrypoints
pruneguard debug runtime                        # Print binary/platform info
pruneguard daemon start|stop|status             # Manage the background daemon
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
# Review your branch (the everyday command)
pruneguard --changed-since origin/main

# Full scan without baseline influence (deterministic CI)
pruneguard --no-baseline --no-cache scan

# Focus to a slice of the repo (full analysis, filtered output)
pruneguard --focus "src/**" scan

# Fail if the scan would be partial-scope
pruneguard --require-full-scope scan

# JSON output for CI pipelines
pruneguard --format json

# SARIF for GitHub Code Scanning
pruneguard --format sarif scan > results.sarif

# Graphviz DOT output
pruneguard --format dot scan | dot -Tsvg -o graph.svg

# Blast radius for a file
pruneguard impact src/utils/helpers.ts

# Explain a specific finding
pruneguard explain unused-export:packages/core:src/old.ts#deprecatedFn

# Check if files are safe to delete
pruneguard safe-delete src/utils/old-helper.ts src/legacy/widget.ts

# Debug module resolution
pruneguard debug resolve ./utils --from src/index.ts
```

## JS API

Every function spawns the native binary and returns parsed, typed results.
See [docs/js-api.md](docs/js-api.md) for the complete API reference.

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
| `review` | `(options?) => Promise<ReviewReport>` | Review your repo or branch |
| `scan` | `(options?) => Promise<AnalysisReport>` | Full repo scan with detailed findings |
| `safeDelete` | `(options) => Promise<SafeDeleteReport>` | Check if files or exports are safe to remove |
| `fixPlan` | `(options) => Promise<FixPlanReport>` | Generate a remediation plan |
| `impact` | `(options) => Promise<ImpactReport>` | Analyze blast radius for a target |
| `explain` | `(options) => Promise<ExplainReport>` | Explain a finding with proof chain |
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
        run: npx pruneguard --changed-since origin/main --format json
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

Most repos work without a config file. Run `pruneguard init` to generate a
minimal config with just the `$schema` reference for editor autocomplete.

For repos that need customization:

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

## Performance

pruneguard is a compiled Rust binary powered by [oxc](https://oxc.rs) for
parsing and module resolution. No V8 or Node.js runtime is on the hot path.

| Scenario | Latency |
|---|---|
| Warm daemon (`review`, `impact`, `explain`) on a small repo | < 10 ms |
| Warm daemon on a medium repo (1,000-2,000 files) | < 50 ms |
| Warm daemon on a large repo (5,000+ files) | < 150 ms |
| Cold one-shot scan, medium repo | < 2 s |
| Cold one-shot scan, 5,000+ file repo | < 6 s |

JS-based alternatives typically take 30-60 seconds on the same large
repositories -- making them too slow for an AI agent to call on every code
change or for a tight inner development loop. pruneguard's speed means you can
treat dead-code detection as a routine check rather than a scheduled chore.

See [docs/performance.md](docs/performance.md) and
[docs/benchmarks.md](docs/benchmarks.md) for methodology and detailed numbers.

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
