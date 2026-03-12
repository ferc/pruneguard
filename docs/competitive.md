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
