# pruneguard

Repo truth engine for JS/TS monorepos.

Build one accurate repo graph, then answer many high-value repo questions cheaply:
unused exports, unused files, unused dependencies, cycles, boundary violations,
ownership visibility, blast-radius analysis, and CI/agent-safe refactor checks.

## Install

```sh
npm install pruneguard
```

The `pruneguard` package automatically installs the correct platform-specific binary.
No Rust toolchain or local compilation required.

## CLI Usage

```sh
# Full scan
pruneguard scan

# Focus findings to a slice of the repo
pruneguard --focus "src/**" scan

# Changed-since review for CI/agents
pruneguard --changed-since origin/main scan

# Deterministic CI/parity run without baseline influence
pruneguard --no-baseline --no-cache scan

# Fail advisory dead-code scans in automation
pruneguard --require-full-scope scan

# Partial-scope scan (advisory for dead-code findings)
pruneguard scan src/components/Button.tsx src/lib/utils.ts

# With config
pruneguard --config pruneguard.json scan

# Blast radius
pruneguard impact src/utils/helpers.ts

# Explain a finding
pruneguard explain unused-export:packages/core:src/old.ts#deprecatedFn

# Generate config
pruneguard init

# Debug resolution
pruneguard debug resolve ./utils --from src/index.ts

# Debug runtime/install info
pruneguard debug runtime
```

## JS API

```js
import { scan, impact, explain, run, binaryPath, loadConfig, schemaPath } from "pruneguard";

// Full scan
const report = await scan({ cwd: "/path/to/repo" });
console.log(report.summary.totalFindings);

// Impact analysis
const blast = await impact({ target: "src/utils/helpers.ts" });
console.log(blast.affectedEntrypoints);

// Explain a finding
const proof = await explain({ query: "src/old.ts#deprecatedFn" });
console.log(proof.proofs);

// Run arbitrary CLI args
const result = await run(["--format", "json", "scan"]);
console.log(result.exitCode);

// Resolve binary path (for custom integrations)
console.log(binaryPath());

// Load resolved config
const config = await loadConfig();

// Additional helpers
import { scanDot, migrateKnip, migrateDepcruise } from "pruneguard";

const dot = await scanDot();                    // Graphviz DOT output
const knipMigration = await migrateKnip();      // Migrate from knip config
const dcMigration = await migrateDepcruise();   // Migrate from dependency-cruiser
```

## Daily-Use Workflows

### Branch scan with `--changed-since`

```sh
pruneguard --changed-since origin/main scan
```

Only reports findings related to files changed on the current branch.

### CI with `--no-baseline`

```sh
pruneguard --no-baseline --no-cache --format json scan
```

Deterministic, no prior-state influence. Use exit code to gate merges.

### Safe deletion review

```sh
# 1. Scan for unused code
pruneguard --format json scan > report.json

# 2. Check blast radius of a candidate removal
pruneguard impact src/utils/old-helper.ts

# 3. Understand why something is live or unused
pruneguard explain src/utils/old-helper.ts
```

## Trust Model

- Full-repo `scan` is the trustworthy mode for deletion decisions
- `--focus` filters reported findings after full analysis
- Positional `scan <paths...>` narrows the analyzed file set and is reported as partial-scope/advisory
- `--require-full-scope` turns advisory partial-scope dead-code scans into a hard failure
- `--no-baseline` disables baseline auto-discovery for deterministic CI, parity, and benchmarks
- Use `impact` and `explain` before removing code on repos with many unresolved specifiers
- Findings carry `confidence` (high, medium, low) to indicate trustworthiness

## Configuration

Create `pruneguard.json`:

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

## Development

Requires: Rust (stable), Node.js, pnpm, just

```sh
just ready          # fmt + check + test + lint
just build          # release binary
just run scan       # run against current directory
just build-js       # build JS wrapper
just schemas        # regenerate shipped schemas
just schemas-check  # verify schemas are committed
just stage-release  # stage npm packages into .release/
just pack-smoke     # end-to-end package install smoke test
just benchmark ../../claude-attack
just benchmark-repos
just smoke-repos    # opt-in real-repo smoke
just parity         # opt-in real-repo parity check
```

## License

MIT
