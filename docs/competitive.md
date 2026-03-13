# Competitive Positioning

pruneguard is a Rust-native repo truth engine for JS/TS monorepos. It competes
with **knip** (unused code finder) and **dependency-cruiser** (dependency graph
validator). This document describes where pruneguard is stronger, where each
competitor excels, and why pruneguard is the best choice for CI pipelines and
AI-agent workflows.

## Feature Comparison

| Capability                          | pruneguard       | knip             | dependency-cruiser |
|-------------------------------------|------------------|------------------|--------------------|
| Unused files                        | Yes              | Yes              | Limited            |
| Unused exports                      | Yes              | Yes              | No                 |
| Unused dependencies                 | Yes              | Yes              | Partial            |
| Cycle detection                     | Yes              | No               | Yes                |
| Dependency graph rules              | Yes              | No               | Yes                |
| Boundary enforcement                | Yes              | No               | Yes                |
| Confidence scoring                  | Yes (3-tier)     | No               | No                 |
| Trust summary                       | Yes              | No               | No                 |
| Branch review gate                  | `review`         | Manual scripting | Manual scripting   |
| Safe-delete evaluation              | `safe-delete`    | No               | No                 |
| Fix-plan remediation                | `fix-plan`       | No               | No                 |
| Impact / blast-radius analysis      | `impact`         | No               | No                 |
| Finding explanations                | `explain`        | No               | No                 |
| SARIF output                        | Yes              | No               | No                 |
| JSON output (machine-readable)      | Yes              | Yes              | Yes                |
| Deterministic ordering              | Yes              | No               | Partial            |
| Ownership / CODEOWNERS integration  | Yes              | No               | No                 |
| Config migration from competitors   | Yes              | N/A              | N/A                |
| Node.js required at runtime         | No (Rust binary) | Yes              | Yes                |
| Framework plugin ecosystem          | Growing          | Mature           | Mature             |

## vs knip

### Where pruneguard wins

**Performance.** pruneguard is compiled to a native Rust binary. Graph
construction, resolution, and analysis all run in Rust without a V8 runtime.
On medium-to-large monorepos this translates to wall-clock times measured in
low tens of milliseconds (warm daemon) compared to seconds in knip.

**Trust-scored findings.** Every finding carries a `confidence` level (high,
medium, low). The report includes a `trust` summary with `unresolvedPressure`,
`fullScope`, and `baselineApplied` flags. Agents and CI scripts can make
data-driven decisions about whether to block a merge or flag for human review
instead of treating all findings equally.

**Machine-actionable remediation.** `fix-plan` generates a structured
remediation plan that machines can execute. `safe-delete` evaluates targets
for deletion safety with confidence and reasons. `review` classifies findings
as blocking vs advisory for branch gating. These commands exist so that an
agent never needs to parse free-text output.

**Architectural boundary enforcement.** pruneguard combines unused-code
analysis with graph-based boundary rules (reachability, tag constraints,
workspace isolation) in a single tool. knip focuses exclusively on unused-code
detection.

**Deterministic results.** Finding IDs, ordering, and evidence are
deterministic across runs. This is essential for baseline comparison, CI
gating, and machine-driven workflows where flaky ordering breaks automation.

### Where knip excels

**Framework and plugin coverage.** knip supports a wide range of JavaScript
frameworks (Angular, Gatsby, Storybook, etc.) through a plugin ecosystem.
pruneguard's framework detection is growing but does not yet match knip's
breadth.

**Community adoption.** knip has a larger user base and more community-authored
documentation and integrations.

## vs dependency-cruiser

### Where pruneguard wins

**Unified analysis.** dependency-cruiser validates dependency rules but does
not detect unused files, unused exports, or unused dependencies. pruneguard
combines all of these analyses with graph-based rules in one tool and one
graph build.

**Native performance.** dependency-cruiser runs on Node.js and requires the
full V8 runtime. pruneguard ships as a self-contained Rust binary with no
Node.js runtime dependency.

**Trust scoring.** dependency-cruiser rules produce pass/fail results.
pruneguard attaches confidence levels to every finding and exposes unresolved
pressure metrics so that consumers can distinguish high-certainty violations
from speculative ones.

**Agent-native output.** pruneguard's `review`, `fix-plan`, `safe-delete`,
`impact`, and `explain` commands produce structured output designed for
machine consumption. dependency-cruiser requires custom reporters or manual
parsing to achieve similar integration.

**Config migration.** `pruneguard migrate depcruise` reads existing
dependency-cruiser config and produces a pruneguard-native configuration,
lowering the barrier to switching.

### Where dependency-cruiser excels

**Rule authoring ecosystem.** dependency-cruiser has a mature rule definition
language with extensive documentation and community-contributed rule sets.

**Wider adoption.** dependency-cruiser is well-established in many
organizations for architectural governance.

## Strictly better for agents and CI

For automated pipelines (GitHub Actions, CI bots, AI coding agents),
pruneguard provides capabilities that neither knip nor dependency-cruiser
offer:

1. **Deterministic JSON output.** Findings are ordered deterministically and
   carry stable IDs. No flaky test failures from ordering changes.

2. **Confidence scoring.** Agents can filter or escalate based on confidence
   level rather than treating every finding as equally trustworthy.

3. **`review` for branch gating.** A single command produces blocking vs
   advisory findings with a trust summary. Exit code 0 means safe to merge.

4. **`fix-plan` for machine-actionable remediation.** Structured output that
   a bot can execute directly instead of parsing human-readable suggestions.

5. **`safe-delete` for deletion approval.** Evaluates targets as safe,
   needsReview, or blocked with confidence and reasons. Returns a recommended
   deletion order.

6. **Trust summaries.** Reports include `fullScope`, `baselineApplied`,
   `unresolvedPressure`, and `confidenceCounts` so that the consumer can
   assess how much to trust the overall result.

7. **SARIF output.** Findings can be emitted in SARIF format for GitHub Code
   Scanning integration, feeding results directly into the GitHub security
   tab.

8. **No runtime dependency.** The binary runs without Node.js, simplifying CI
   container images and reducing install time.

## Intentional Differences

pruneguard intentionally diverges from both knip and dependency-cruiser in
several design areas. These are not gaps -- they are deliberate choices.

### vs knip

| Area | knip behavior | pruneguard behavior | Rationale |
|------|--------------|---------------------|-----------|
| Finding identity | No stable finding IDs | Deterministic `id` per finding | Machine workflows need stable IDs for baseline diff, caching, and deduplication |
| Confidence scoring | All findings are equally weighted | Each finding carries `high`, `medium`, or `low` confidence based on unresolved import pressure | Agents and CI should distinguish certain dead code from speculative results |
| Type vs value liveness | Reports unused exports as a single category | Separates type-only and value liveness in unused-export analysis | TypeScript `import type` should not keep a value export alive |
| Baseline format | Dedicated `.knip.json` baseline schema | Treats `baseline.json` as a prior `AnalysisReport` | Reusing the report format means any scan output can serve as a baseline without schema translation |
| Ambient declarations | `.d.ts` files may be flagged | `.d.ts` files are categorically excluded from dead-code findings | Ambient declarations are project infrastructure, not dead code |
| Script-level dependency usage | May miss script-only usage | Scans `package.json` scripts for direct dependency references before reporting unused deps | `"build": "tsc"` means `typescript` is used |
| Runtime prefixes | Varies | `node:`, `bun:`, `deno:` prefixed specifiers are classified as externalized, not unresolved | Built-in module prefixes are not installation failures |

### vs dependency-cruiser

| Area | dependency-cruiser behavior | pruneguard behavior | Rationale |
|------|----------------------------|---------------------|-----------|
| Analysis scope | Dependency graph rules only | Unified dead-code detection + graph rules in one tool and one graph build | One tool, one graph, one invocation |
| Rule output | Pass/fail per rule | Each finding carries confidence, evidence chain, and finding ID | Machine consumers need structured, granular output |
| Config migration | N/A | `pruneguard migrate depcruise` reads `.dependency-cruiser.mjs` and produces pruneguard config | Lowers switching cost |
| Ownership | Not supported | CODEOWNERS integration with cross-owner violation detection | Architecture governance includes team boundaries |

## Performance Comparison Methodology

pruneguard benchmarks are designed to be reproducible, fair, and auditable.

### Setup

1. **Corpora.** Performance is measured against canonical external repositories
   defined in `benchmarks/corpora.toml`: knip, dependency-cruiser, oxc, and
   claude-attack. Each corpus has a known minimum file and package count to
   detect regressions.

2. **Machine configuration.** Benchmarks are run on a single machine with
   controlled background load. Results include the machine architecture and
   OS version for reproducibility.

3. **Warm vs cold.** Both cold (no cache, no daemon) and warm (daemon running,
   cache populated) timings are collected. Cold timing uses
   `--no-cache --daemon off`. Warm timing uses the default daemon mode after
   a priming scan.

### Measurement

1. **Wall-clock time.** Primary metric. Measured via `hyperfine` (minimum 5
   runs, warmup of 1 run for cold, 3 runs for warm).

2. **Peak RSS.** Measured via `/usr/bin/time -l` (macOS) or
   `/usr/bin/time -v` (Linux).

3. **Determinism check.** After timing, two consecutive scans are compared
   for finding-ID and ordering stability. Non-deterministic output would
   invalidate the benchmark.

### Comparison protocol

When comparing against knip or dependency-cruiser:

1. Both tools analyze the same corpus at the same git commit.
2. Both tools run with their default configuration (no custom plugins or
   rules) unless the comparison specifically tests a feature that requires
   configuration.
3. knip is run via `npx knip --reporter json`. dependency-cruiser is run
   via `npx depcruise --output-type json src/`.
4. pruneguard is run via `pruneguard --format json --no-cache --daemon off scan`.
5. Timing excludes npm/npx startup overhead for the JS tools (measured
   separately and noted).
6. Results report: wall-clock time, peak RSS, number of findings, and
   whether the tool completed without errors.

### Reporting

Benchmark results are stored in `benchmarks/results/` as JSON files with
fields: `corpus`, `tool`, `version`, `commit`, `cold_ms`, `warm_ms`,
`peak_rss_kb`, `findings_count`, `timestamp`, and `machine`.

Results are not committed to the main branch -- they are generated locally
and referenced in release notes when relevant.

## Known Noisy Repo Patterns

Certain repository structures produce more findings or lower trust scores
across all tools. pruneguard's confidence scoring surfaces this explicitly
rather than silently over- or under-reporting. The patterns below commonly
cause noise and are worth understanding before acting on findings.

| Pattern | Effect | Mitigation |
|---------|--------|------------|
| Heavy dynamic `require()` / `import()` | Unresolvable specifiers increase `unresolvedPressure`, lowering overall trust | Add the dynamic targets to `entrypoints.custom` or `ignore` in config |
| `module.exports = require(...)` re-export barrels | pruneguard may not trace through CommonJS re-export patterns as deeply as ESM | Use `compatibilityReport` to check for unsupported signals; add barrel files to `entrypoints.custom` |
| Webpack/Vite aliases without tsconfig paths | Specifiers resolve at build time but not at static analysis time | Mirror aliases in `tsconfig.json` `paths` or add them to `resolve.aliases` in pruneguard config |
| Codegen / template-generated files | Generated files may appear unused because the generation step is not traced | Add generated output directories to `ignore` or classify them as `role: "generated"` |
| Monorepos with 100+ workspaces | Graph size can push warm-daemon latency above 100ms and cold scans into multi-second range | Use `--focus` to narrow reporting scope; use partial-scope scans for targeted analysis |
| Heavy decorator-based frameworks (NestJS, Angular) | Decorators create implicit usage that static analysis cannot always trace | Check `compatibilityReport` warnings; add framework entrypoints manually if needed |
| Mixed CJS/ESM packages | Dual-format packages may have specifiers that resolve differently per consumer | Use `debugResolve` to trace specific specifiers; ensure `type` field in package.json is correct |
| Plugin architectures with string-based loading | Plugins loaded by string name (e.g., Babel, ESLint) create invisible edges | Add plugin files to `entrypoints.custom` in config |

When encountering high `unresolvedPressure` (above 5%) or many low-confidence
findings, run `compatibilityReport` to identify specific framework or
toolchain gaps, then consult the table above for targeted mitigations.
