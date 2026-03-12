# pruneguard

**One graph. Every answer.** Find unused exports, dead files, phantom dependencies, import cycles, and boundary violations across your entire JS/TS monorepo — in a single, fast Rust-powered pass.

[![npm version](https://img.shields.io/npm/v/pruneguard)](https://www.npmjs.com/package/pruneguard)
[![license](https://img.shields.io/npm/l/pruneguard)](https://github.com/ferc/pruneguard/blob/main/LICENSE)

---

## Why pruneguard?

Large JS/TS monorepos accumulate dead code, orphan files, and undeclared cross-package imports faster than any team can manually track. Existing tools either focus on a single concern, struggle with monorepo workspaces, or sacrifice accuracy for speed.

Pruneguard builds one complete module graph — resolving TypeScript paths, package.json `exports`, workspace links, and framework conventions — then runs every analyzer over it in a single pass. The result: fast, accurate, actionable findings with proof chains you can verify before deleting a single line.

### Key strengths

- **Rust-native speed** — parses and resolves thousands of files in seconds, powered by [oxc](https://oxc.rs)
- **Monorepo-first** — understands pnpm/npm/yarn/bun workspaces, cross-package imports, and `exports` maps out of the box
- **8 built-in analyzers** — unused exports, unused files, unused packages, unused dependencies, cycles, boundary violations, ownership, and impact analysis
- **Framework-aware** — auto-detects Next.js, Vite, Vitest, Jest, and Storybook entrypoints so you don't over-report
- **Trust model** — every finding carries a confidence level; partial-scope scans are clearly marked as advisory
- **Explainability** — `impact` and `explain` commands let you trace proof chains before acting
- **CI-ready** — SARIF output, deterministic mode, `--changed-since` for incremental PR checks, exit codes for gating
- **Migrate easily** — built-in config converters for knip and dependency-cruiser

---

## Quick start

```sh
# Install
npm install -D pruneguard   # or: npx pruneguard scan

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
  scan [PATHS...]        Analyze the repo (default command)
  impact <TARGET>        Compute blast radius for a file or export
  explain <QUERY>        Show proof chain for a finding, file, or export
  init                   Generate a default pruneguard.json
  print-config           Display the resolved configuration
  debug resolve          Debug module resolution
  debug entrypoints      List detected entrypoints
  debug runtime          Print runtime diagnostics
  migrate knip           Convert knip config to pruneguard
  migrate depcruise      Convert dependency-cruiser config to pruneguard

Options:
  -c, --config <FILE>          Config file path [default: pruneguard.json]
      --format <FORMAT>        Output format: text, json, sarif, dot
      --profile <PROFILE>      Analysis profile: production, development, all
      --changed-since <REF>    Only analyze files changed since a git ref
      --focus <GLOB>           Filter findings to matching files
      --severity <SEVERITY>    Minimum severity: error, warn, info
      --no-cache               Disable incremental cache
      --no-baseline            Disable baseline suppression
      --require-full-scope     Fail if scan is partial-scope
      --max-findings <N>       Cap the number of reported findings
  -V, --version                Print version
  -h, --help                   Print help
```

### Common workflows

```sh
# Full scan with JSON output for CI
pruneguard scan --format json

# PR check — only findings from changed files
pruneguard --changed-since origin/main scan

# Deterministic CI (no cache, no baseline)
pruneguard --no-baseline --no-cache scan

# Focus on a specific area without narrowing analysis scope
pruneguard --focus "packages/core/**" scan

# SARIF for GitHub Code Scanning
pruneguard scan --format sarif > results.sarif

# Visualize the module graph
pruneguard scan --format dot | dot -Tsvg -o graph.svg
```

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

  "entrypoints": {
    "auto": true,
    "include": ["src/index.ts"],
    "exclude": ["**/*.test.ts"]
  },

  "analysis": {
    "unusedExports": "error",
    "unusedFiles": "warn",
    "unusedDependencies": "error",
    "unusedPackages": "warn",
    "cycles": "warn",
    "boundaries": "error"
  },

  "frameworks": {
    "next": "auto",
    "vitest": "auto",
    "storybook": "auto"
  },

  "rules": {
    "forbidden": [
      {
        "name": "no-cross-app-imports",
        "severity": "error",
        "comment": "Apps must not import from other apps",
        "from": { "workspace": ["apps/*"] },
        "to": { "workspace": ["apps/*"] }
      }
    ]
  },

  "ownership": {
    "importCodeowners": true,
    "unownedSeverity": "warn"
  }
}
```

Full schema reference is bundled at `node_modules/pruneguard/configuration_schema.json` — editors with JSON Schema support will provide autocomplete and validation automatically.

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
| **Impact** | — | Reverse-reachability blast radius (via `pruneguard impact`) |

Each finding includes a **confidence level** (high / medium / low) based on analysis scope and unresolved-specifier pressure, so you always know how much to trust a result.

---

## Trust model

Pruneguard is designed for safe, incremental adoption — not surprise bulk deletions.

| Mode | Behavior | Use case |
|---|---|---|
| `pruneguard scan` | Full-repo analysis, high-confidence findings | Deletion decisions, CI gating |
| `--focus "glob"` | Full analysis, findings filtered to matching files | Scoping reports to a team or area |
| `scan <paths...>` | Partial-scope, findings marked advisory | Quick local checks |
| `--changed-since ref` | Incremental, only changed files analyzed | PR review, fast CI |
| `--require-full-scope` | Fails if scan would be partial-scope | Strict CI enforcement |
| `--no-baseline` | No baseline suppression | Deterministic CI, benchmarks |

**Recommended deletion flow:**

1. `pruneguard scan --format json` — identify candidates
2. `pruneguard impact <target>` — check blast radius
3. `pruneguard explain <finding>` — review proof chain
4. Delete with confidence

---

## Programmatic API

```ts
import { scan, impact, explain, loadConfig } from "pruneguard";

// Scan and get structured results
const report = await scan({
  profile: "production",
  changedSince: "origin/main",
});

console.log(`${report.summary.totalFindings} findings`);

// Blast radius analysis
const blast = await impact({ target: "src/utils/helpers.ts" });
console.log(`Affects ${blast.affectedEntrypoints.length} entrypoints`);

// Explain a finding
const proof = await explain({
  query: "unused-export:packages/core:src/old.ts#deprecatedFn",
});
```

Full TypeScript types are included via `dist/index.d.mts`.

---

## Framework detection

Pruneguard auto-detects popular frameworks and registers their entrypoints and file conventions, so test files, stories, and framework config files aren't flagged as unused.

| Framework | Auto-detected via | Entrypoints added |
|---|---|---|
| **Next.js** | `next` dependency, `next.config.*` | `app/page.*`, `app/layout.*`, `pages/**` |
| **Vite** | `vite` devDependency, `vite.config.*` | `vite.config.*` |
| **Vitest** | `vitest` devDependency | `vitest.config.*`, `**/*.test.*` |
| **Jest** | `jest` devDependency | `jest.config.*`, `**/*.test.*` |
| **Storybook** | `@storybook/*` packages | `.storybook/main.*`, `**/*.stories.*` |

Override with `"frameworks": { "next": "off" }` in config.

---

## Migrating from other tools

```sh
# From knip
pruneguard migrate knip

# From dependency-cruiser
pruneguard migrate depcruise
```

Both commands read your existing config and emit an equivalent `pruneguard.json` with migration notes.

---

## Output formats

| Format | Flag | Use case |
|---|---|---|
| **Text** | `--format text` | Human-readable terminal output (default) |
| **JSON** | `--format json` | CI pipelines, scripts, programmatic consumption |
| **SARIF** | `--format sarif` | GitHub Code Scanning, Azure DevOps, IDE integrations |
| **DOT** | `--format dot` | Graph visualization with Graphviz |

---

## Requirements

- Node.js >= 18
- Supported platforms: macOS (ARM64, x64), Linux (x64 glibc/musl, ARM64 glibc/musl), Windows (x64, ARM64)

---

## License

[MIT](https://github.com/ferc/pruneguard/blob/main/LICENSE)
