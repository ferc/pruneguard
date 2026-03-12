# pruneguard

**One graph. Every answer.** Find unused exports, dead files, phantom dependencies, import cycles, and boundary violations across your entire JS/TS monorepo -- in a single, fast Rust-powered pass.

[![npm version](https://img.shields.io/npm/v/pruneguard)](https://www.npmjs.com/package/pruneguard)
[![license](https://img.shields.io/npm/l/pruneguard)](https://github.com/ferc/pruneguard/blob/main/LICENSE)

---

## Install

```sh
npm install -D pruneguard   # or: npx pruneguard scan
```

The package automatically installs the correct platform-specific native binary. No Rust toolchain, no compilation, no native addons -- just `npm install` and go.

**Supported platforms:** macOS (ARM64, x64), Linux (x64/ARM64, glibc and musl), Windows (x64, ARM64). Requires Node.js >= 18.

### How it works

pruneguard ships a compiled Rust binary for each platform. The JS API and CLI both spawn this binary. Locally the daemon keeps the graph warm for instant queries. In CI every invocation is a fresh one-shot run.

---

## Quick start

```sh
# Scan your repo
pruneguard scan

# See what breaks if you touch a file
pruneguard impact src/utils/helpers.ts

# Understand why something is flagged
pruneguard explain unused-export:packages/core:src/old.ts#deprecatedFn

# Generate a config file
pruneguard init
```

---

## CLI reference

```
pruneguard [OPTIONS] <COMMAND>

Commands:
  scan [PATHS...]             Analyze the repo (default command)
  impact <TARGET>             Compute blast radius for a file or export
  explain <QUERY>             Show proof chain for a finding, file, or export
  review                      Branch review gate (blocking vs advisory findings)
  safe-delete <TARGETS...>    Evaluate targets for safe deletion
  fix-plan <TARGETS...>       Generate structured remediation plan
  suggest-rules               Auto-suggest governance rules from graph analysis
  init                        Generate a default pruneguard.json
  print-config                Display the resolved configuration
  debug resolve               Debug module resolution
  debug entrypoints           List detected entrypoints
  debug runtime               Print runtime diagnostics
  daemon start|stop|status    Manage the background daemon
Options:
  -c, --config <FILE>          Config file path [default: pruneguard.json]
      --format <FORMAT>        Output format: text, json, sarif, dot
      --profile <PROFILE>      Analysis profile: production, development, all
      --changed-since <REF>    Only report findings for changed files
      --focus <GLOB>           Filter findings to matching files
      --severity <SEVERITY>    Minimum severity: error, warn, info
      --no-cache               Disable incremental cache
      --no-baseline            Disable baseline suppression
      --require-full-scope     Fail if scan is partial-scope
      --max-findings <N>       Cap the number of reported findings
      --daemon <MODE>          Daemon mode: auto, off, required
  -V, --version                Print version
  -h, --help                   Print help
```

### Common workflows

```sh
# Full scan with JSON output for CI
pruneguard --format json scan

# PR check -- only findings from changed files
pruneguard --changed-since origin/main scan

# Deterministic CI (no cache, no baseline)
pruneguard --no-baseline --no-cache scan

# Branch review gate
pruneguard --changed-since origin/main review

# Safe-delete check before cleanup
pruneguard safe-delete src/old.ts src/legacy/widget.ts

# Focus on a specific area without narrowing analysis scope
pruneguard --focus "packages/core/**" scan

# SARIF for GitHub Code Scanning
pruneguard --format sarif scan > results.sarif

# Visualize the module graph
pruneguard --format dot scan | dot -Tsvg -o graph.svg
```

---

## Programmatic API

All functions spawn the native binary and return typed results.

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
  scanDot,
} from "pruneguard";
```

### scan

```js
const report = await scan({
  profile: "production",
  changedSince: "origin/main",
});
console.log(`${report.summary.totalFindings} findings`);
```

### review

```js
const result = await review({ baseRef: "origin/main", noCache: true });
if (result.blockingFindings.length > 0) {
  console.error("Blocking findings exist");
  process.exit(1);
}
```

### safeDelete

```js
const result = await safeDelete({ targets: ["src/old.ts"] });
console.log("Safe:", result.safe.map(e => e.target));
console.log("Blocked:", result.blocked.map(e => e.target));
```

### fixPlan

```js
const plan = await fixPlan({ targets: ["src/old.ts"] });
for (const action of plan.actions) {
  console.log(`${action.kind}: ${action.targets.join(", ")}`);
}
```

### impact

```js
const blast = await impact({ target: "src/utils/helpers.ts" });
console.log(`Affects ${blast.affectedEntrypoints.length} entrypoints`);
```

### explain

```js
const proof = await explain({ query: "src/old.ts#deprecatedFn" });
console.log(proof.proofs);
```

### run

```js
const result = await run(["--format", "json", "--daemon", "off", "scan"]);
console.log(result.exitCode, result.durationMs);
```

### binaryPath

```js
console.log(binaryPath());
// => /path/to/node_modules/@pruneguard/cli-darwin-arm64/bin/pruneguard
```

### Other functions

```js
const config = await loadConfig();
const schema = schemaPath();
const dot = await scanDot();
const rules = await suggestRules();
```

### Error handling

All functions throw `PruneguardExecutionError` with a `code` field:

| Code | Meaning |
|---|---|
| `PRUNEGUARD_BINARY_NOT_FOUND` | Native binary not found |
| `PRUNEGUARD_EXECUTION_FAILED` | Binary exited with unexpected code |
| `PRUNEGUARD_JSON_PARSE_FAILED` | Output was not valid JSON |

```js
import { scan, PruneguardExecutionError } from "pruneguard";

try {
  await scan();
} catch (err) {
  if (err instanceof PruneguardExecutionError) {
    console.error(err.code, err.message);
  }
}
```

Full TypeScript types are included via `dist/index.d.mts`.

---

## Configuration

Create `pruneguard.json` (or `.pruneguardrc.json`) in your project root. Run `pruneguard init` to generate one.

```jsonc
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json",

  "workspaces": {
    "roots": ["apps/*", "packages/*"],
    "packageManager": "pnpm"
  },

  "analysis": {
    "unusedExports": "error",
    "unusedFiles": "warn",
    "unusedDependencies": "error",
    "cycles": "warn"
  },

  "frameworks": {
    "next": "auto",
    "vitest": "auto",
    "storybook": "auto"
  }
}
```

Full schema reference is bundled at `node_modules/pruneguard/configuration_schema.json`.

---

## Analyzers

| Analyzer | Config key | What it finds |
|---|---|---|
| **Unused exports** | `unusedExports` | Named/default exports never imported by reachable code |
| **Unused files** | `unusedFiles` | Source files unreachable from any entrypoint |
| **Unused packages** | `unusedPackages` | Workspace packages with zero reachable files |
| **Unused dependencies** | `unusedDependencies` | Declared `dependencies` never referenced by reachable code |
| **Cycles** | `cycles` | Circular dependency chains (strongly connected components) |
| **Boundary violations** | `boundaries` | Custom forbidden/required import rules |
| **Ownership** | `ownership` | Files without a matching team in CODEOWNERS or config |
| **Impact** | -- | Reverse-reachability blast radius (via `pruneguard impact`) |

Each finding includes a **confidence level** (high / medium / low) so you always know how much to trust a result.

---

## Trust model

| Mode | Behavior | Use case |
|---|---|---|
| `pruneguard scan` | Full-repo analysis, high-confidence findings | Deletion decisions, CI gating |
| `--focus "glob"` | Full analysis, findings filtered to matching files | Scoping reports to a team or area |
| `scan <paths...>` | Partial-scope, findings marked advisory | Quick local checks |
| `--changed-since ref` | Full graph, only changed-file findings reported | PR review, fast CI |
| `--require-full-scope` | Fails if scan would be partial-scope | Strict CI enforcement |
| `--no-baseline` | No baseline suppression | Deterministic CI, benchmarks |

---

## GitHub Actions

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

### Baseline-gated CI

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

      - name: Save baseline
        if: github.ref == 'refs/heads/main'
        run: npx pruneguard --no-cache --no-baseline --format json scan > baseline.json

      - uses: actions/upload-artifact@v6
        if: github.ref == 'refs/heads/main'
        with:
          name: pruneguard-baseline
          path: baseline.json

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
            if (!fs.existsSync('baseline.json')) { console.log('No baseline, skipping'); process.exit(0); }
            const base = JSON.parse(fs.readFileSync('baseline.json','utf-8'));
            const curr = JSON.parse(fs.readFileSync('current.json','utf-8'));
            const ids = new Set(base.findings.map(f => f.id));
            const n = curr.findings.filter(f => !ids.has(f.id));
            if (n.length) { n.forEach(f => console.error(f.id+': '+f.message)); process.exit(1); }
            console.log('No new findings.');
          "
```

---

## Framework detection

| Framework | Auto-detected via | Entrypoints added |
|---|---|---|
| **Next.js** | `next` dependency, `next.config.*` | `app/page.*`, `app/layout.*`, `pages/**` |
| **Vite** | `vite` devDependency, `vite.config.*` | `vite.config.*` |
| **Vitest** | `vitest` devDependency | `vitest.config.*`, `**/*.test.*` |
| **Jest** | `jest` devDependency | `jest.config.*`, `**/*.test.*` |
| **Storybook** | `@storybook/*` packages | `.storybook/main.*`, `**/*.stories.*` |

Override with `"frameworks": { "next": "off" }` in config.

---

## Requirements

- Node.js >= 18
- Supported platforms: macOS (ARM64, x64), Linux (x64 glibc/musl, ARM64 glibc/musl), Windows (x64, ARM64)

---

## License

[MIT](https://github.com/ferc/pruneguard/blob/main/LICENSE)
