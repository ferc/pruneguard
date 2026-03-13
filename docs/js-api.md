# JS API Reference

Every function spawns the native pruneguard binary and returns parsed, typed
results. No Rust toolchain is required at runtime.

```sh
npm install pruneguard
```

```js
import { review, scan, safeDelete, fixPlan, impact, explain, binaryPath, run } from "pruneguard";
```

The primary functions for daily use are `review`, `scan`, `safeDelete`, and `fixPlan`. Advanced functions like `suggestRules`, `compatibilityReport`, `debugFrameworks`, and migration helpers are documented below.

---

## review

Branch review gate. Classifies findings as blocking (high-confidence
errors/warnings) or advisory. Exit code 0 means safe to merge.

```ts
function review(options?: ReviewOptions): Promise<ReviewReport>
```

### ReviewOptions

| Field        | Type      | Default         | Description |
|--------------|-----------|-----------------|-------------|
| `cwd`        | `string`  | `process.cwd()` | Working directory |
| `config`     | `string`  |                 | Config file path |
| `profile`    | `Profile` |                 | Analysis profile |
| `baseRef`    | `string`  |                 | Git ref for changed-since filtering |
| `noCache`    | `boolean` | `false`         | Disable incremental cache |
| `noBaseline` | `boolean` | `false`         | Disable baseline suppression |

### ReviewReport

```ts
{
  baseRef?: string;
  changedFiles: string[];
  newFindings: Array<Finding>;
  blockingFindings: Array<Finding>;
  advisoryFindings: Array<Finding>;
  trust: {
    fullScope: boolean;
    baselineApplied: boolean;
    unresolvedPressure: number;
    confidenceCounts: { high: number; medium: number; low: number };
  };
  recommendations: string[];
  proposedActions?: Array<Action>;
  executionMode?: "oneshot" | "daemon";
  latencyMs?: number;
}
```

### Example

```js
import { review } from "pruneguard";

const result = await review({
  baseRef: "origin/main",
  noCache: true,
});

if (result.blockingFindings.length > 0) {
  for (const f of result.blockingFindings) {
    console.error(`[${f.confidence}] ${f.code}: ${f.message}`);
  }
  process.exit(1);
}

console.log("Branch is clean. Safe to merge.");
```

---

## scan

Run a full or partial-scope analysis of the repository.

```ts
function scan(options?: ScanOptions): Promise<AnalysisReport>
```

### ScanOptions

| Field              | Type       | Default         | Description |
|--------------------|------------|-----------------|-------------|
| `cwd`              | `string`   | `process.cwd()` | Working directory |
| `config`           | `string`   |                 | Config file path |
| `paths`            | `string[]` |                 | Partial-scope: only analyze these paths |
| `profile`          | `Profile`  |                 | `"production"`, `"development"`, or `"all"` |
| `changedSince`     | `string`   |                 | Git ref for changed-file filtering |
| `focus`            | `string`   |                 | Glob to filter reported findings |
| `noCache`          | `boolean`  | `false`         | Disable incremental cache |
| `noBaseline`       | `boolean`  | `false`         | Disable baseline suppression |
| `requireFullScope` | `boolean`  | `false`         | Fail if scan would be partial-scope |

### AnalysisReport

```ts
{
  version: number;
  toolVersion: string;
  cwd: string;
  profile: string;
  summary: {
    totalFiles: number;
    totalPackages: number;
    totalWorkspaces: number;
    totalExports: number;
    totalFindings: number;
    errors: number;
    warnings: number;
    infos: number;
  };
  inventories: {
    files: Array<{ path: string; workspace?: string; kind: string; role?: string }>;
    packages: Array<{ name: string; version?: string; workspace: string; path: string }>;
    workspaces: Array<{ name: string; path: string; packageCount: number }>;
  };
  findings: Array<Finding>;
  entrypoints: Array<{ path: string; kind: string; profile: string; workspace?: string; source: string }>;
  stats: Stats;
}
```

### Example

```js
import { scan } from "pruneguard";

const report = await scan({
  cwd: "/path/to/repo",
  profile: "production",
  changedSince: "origin/main",
  noCache: true,
});

console.log(report.summary.totalFindings);

for (const f of report.findings) {
  console.log(`[${f.severity}] [${f.confidence}] ${f.code}: ${f.message}`);
}
```

---

## safeDelete

Evaluate targets for safe deletion. Each target is classified as safe,
needs-review, or blocked with confidence levels and reasons.

```ts
function safeDelete(options: SafeDeleteOptions): Promise<SafeDeleteReport>
```

### SafeDeleteOptions

| Field     | Type       | Default         | Description |
|-----------|------------|-----------------|-------------|
| `cwd`     | `string`   | `process.cwd()` | Working directory |
| `config`  | `string`   |                 | Config file path |
| `profile` | `Profile`  |                 | Analysis profile |
| `targets` | `string[]` | **(required)**  | Files to evaluate for deletion |
| `noCache` | `boolean`  | `false`         | Disable incremental cache |

### SafeDeleteReport

```ts
{
  targets: string[];
  safe: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  needsReview: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  blocked: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  deletionOrder: string[];
  evidence: Array<Evidence>;
}
```

### Example

```js
import { safeDelete } from "pruneguard";

const result = await safeDelete({
  targets: ["src/legacy/old-widget.ts", "src/utils/deprecated-helper.ts"],
});

console.log("Safe:", result.safe.map(e => e.target));
console.log("Blocked:", result.blocked.map(e => `${e.target}: ${e.reasons.join(", ")}`));
console.log("Deletion order:", result.deletionOrder);
```

---

## fixPlan

Generate a structured remediation plan with specific actions, steps, risk
levels, and verification instructions.

```ts
function fixPlan(options: FixPlanOptions): Promise<FixPlanReport>
```

### FixPlanOptions

| Field     | Type       | Default         | Description |
|-----------|------------|-----------------|-------------|
| `cwd`     | `string`   | `process.cwd()` | Working directory |
| `config`  | `string`   |                 | Config file path |
| `profile` | `Profile`  |                 | Analysis profile |
| `targets` | `string[]` | **(required)**  | Finding IDs or file paths to plan fixes for |
| `noCache` | `boolean`  | `false`         | Disable incremental cache |

### FixPlanReport

```ts
{
  query: string[];
  matchedFindings: Array<Finding>;
  actions: Array<{
    id: string;
    kind: string;
    targets: string[];
    why: string;
    preconditions: string[];
    steps: Array<{ description: string; file?: string; action?: string }>;
    verification: string[];
    risk: "low" | "medium" | "high";
    confidence: "high" | "medium" | "low";
  }>;
  blockedBy: string[];
  verificationSteps: string[];
  riskLevel: "low" | "medium" | "high";
  confidence: "high" | "medium" | "low";
}
```

### Example

```js
import { fixPlan } from "pruneguard";

const plan = await fixPlan({
  targets: ["src/legacy/old-widget.ts"],
});

for (const action of plan.actions) {
  console.log(`${action.kind}: ${action.targets.join(", ")} (${action.risk} risk)`);
  for (const step of action.steps) {
    console.log(`  - ${step.description}`);
  }
}
```

---

## impact

Blast-radius analysis for a target file or export. Shows which entrypoints,
packages, and files would be affected by changes to the target.

```ts
function impact(options: ImpactOptions): Promise<ImpactReport>
```

### ImpactOptions

| Field     | Type      | Default         | Description |
|-----------|-----------|-----------------|-------------|
| `cwd`     | `string`  | `process.cwd()` | Working directory |
| `config`  | `string`  |                 | Config file path |
| `target`  | `string`  | **(required)**  | File path or export to analyze |
| `profile` | `Profile` |                 | Analysis profile |
| `focus`   | `string`  |                 | Glob to filter results |

### ImpactReport

```ts
{
  target: string;
  affectedEntrypoints: string[];
  affectedPackages: string[];
  affectedFiles: string[];
  evidence: Array<Evidence>;
  focusFiltered: boolean;
}
```

### Example

```js
import { impact } from "pruneguard";

const blast = await impact({ target: "src/utils/helpers.ts" });

console.log("Affected entrypoints:", blast.affectedEntrypoints.length);
console.log("Affected files:", blast.affectedFiles.length);
console.log("Affected packages:", blast.affectedPackages);
```

---

## explain

Get a proof chain explaining why a file or export is used, unused, or
violating a boundary.

```ts
function explain(options: ExplainOptions): Promise<ExplainReport>
```

### ExplainOptions

| Field     | Type      | Default         | Description |
|-----------|-----------|-----------------|-------------|
| `cwd`     | `string`  | `process.cwd()` | Working directory |
| `config`  | `string`  |                 | Config file path |
| `query`   | `string`  | **(required)**  | Finding ID, file path, or `file#export` |
| `profile` | `Profile` |                 | Analysis profile |
| `focus`   | `string`  |                 | Glob to filter results |

### ExplainReport

```ts
{
  query: string;
  matchedNode?: string;
  queryKind: "finding" | "file" | "export";
  proofs: Array<{
    node: string;
    relationship: string;
    children: Proof[];
  }>;
  relatedFindings: Array<Finding>;
  focusFiltered: boolean;
}
```

### Example

```js
import { explain } from "pruneguard";

const proof = await explain({ query: "src/old.ts#deprecatedFn" });

console.log("Kind:", proof.queryKind);
for (const p of proof.proofs) {
  console.log(`${p.node} -- ${p.relationship}`);
}
```

---

## suggestRules

Auto-suggest governance rules based on your repository's dependency graph
structure.

```ts
function suggestRules(options?: SuggestRulesOptions): Promise<SuggestRulesReport>
```

### SuggestRulesOptions

| Field     | Type      | Default         | Description |
|-----------|-----------|-----------------|-------------|
| `cwd`     | `string`  | `process.cwd()` | Working directory |
| `config`  | `string`  |                 | Config file path |
| `profile` | `Profile` |                 | Analysis profile |
| `noCache` | `boolean` | `false`         | Disable incremental cache |

### SuggestRulesReport

```ts
{
  suggestedRules: Array<{
    kind: string;
    name: string;
    description: string;
    configFragment: Record<string, unknown>;
    confidence: "high" | "medium" | "low";
    evidence?: string[];
  }>;
  tags?: Array<{ name: string; glob: string; rationale: string }>;
  ownershipHints?: Array<{
    pathGlob: string;
    suggestedOwner: string;
    crossTeamEdges: number;
    rationale: string;
  }>;
  hotspots?: Array<{
    file: string;
    crossPackageImports: number;
    crossOwnerImports: number;
    incomingEdges: number;
    outgoingEdges: number;
    suggestion: string;
  }>;
  rationale?: string[];
}
```

### Example

```js
import { suggestRules } from "pruneguard";

const result = await suggestRules();

for (const rule of result.suggestedRules) {
  console.log(`[${rule.confidence}] ${rule.name}: ${rule.description}`);
}
```

---

## compatibilityReport

Check framework and toolchain compatibility. Surfaces unsupported signals,
trust downgrades, and warnings that affect finding accuracy. Run this to
understand why trust scores may be degraded before acting on findings.

```ts
function compatibilityReport(options?: CompatibilityReportOptions): Promise<CompatibilityReport>
```

### CompatibilityReportOptions

| Field     | Type      | Default         | Description |
|-----------|-----------|-----------------|-------------|
| `cwd`     | `string`  | `process.cwd()` | Working directory |
| `config`  | `string`  |                 | Config file path |
| `profile` | `Profile` |                 | Analysis profile |

### CompatibilityReport

```ts
{
  supportedFrameworks: string[];
  heuristicFrameworks: string[];
  unsupportedSignals: Array<{
    signal: string;
    source: string;
    suggestion?: string;
  }>;
  warnings: Array<{
    code: string;
    message: string;
    affectedScope?: string;
    severity: "low" | "medium" | "high";
  }>;
  trustDowngrades: Array<{
    reason: string;
    scope: string;
    severity: "low" | "medium" | "high";
  }>;
}
```

### Example

```js
import { compatibilityReport } from "pruneguard";

const compat = await compatibilityReport();

console.log("Supported:", compat.supportedFrameworks);
console.log("Heuristic:", compat.heuristicFrameworks);

for (const signal of compat.unsupportedSignals) {
  console.warn(`Unsupported: ${signal.signal} (from ${signal.source})`);
  if (signal.suggestion) console.warn(`  Suggestion: ${signal.suggestion}`);
}

for (const downgrade of compat.trustDowngrades) {
  console.warn(`Trust downgrade [${downgrade.severity}]: ${downgrade.reason} (scope: ${downgrade.scope})`);
}
```

---

## debugFrameworks

Show detailed framework detection diagnostics. Lists all detected framework
packs, the entrypoints and ignore patterns they contributed, and which
detections were heuristic vs exact.

```ts
function debugFrameworks(options?: DebugFrameworksOptions): Promise<FrameworkDebugReport>
```

### DebugFrameworksOptions

| Field     | Type      | Default         | Description |
|-----------|-----------|-----------------|-------------|
| `cwd`     | `string`  | `process.cwd()` | Working directory |
| `config`  | `string`  |                 | Config file path |
| `profile` | `Profile` |                 | Analysis profile |

### FrameworkDebugReport

```ts
{
  detectedPacks: Array<{
    name: string;
    confidence: string;
    signals: string[];
    reasons: string[];
  }>;
  allEntrypoints: Array<{
    path: string;
    framework: string;
    kind: string;
    heuristic: boolean;
    reason: string;
  }>;
  allIgnorePatterns: string[];
  allClassificationRules: Array<{
    pattern: string;
    classification: string;
  }>;
  heuristicDetections: string[];
}
```

### Example

```js
import { debugFrameworks } from "pruneguard";

const fwDebug = await debugFrameworks();

for (const pack of fwDebug.detectedPacks) {
  console.log(`${pack.name} (${pack.confidence}): ${pack.signals.join(", ")}`);
}

console.log("Heuristic detections:", fwDebug.heuristicDetections);
console.log("Framework entrypoints:", fwDebug.allEntrypoints.length);
```

---

## loadConfig

Load and return the fully resolved pruneguard configuration.

```ts
function loadConfig(options?: { cwd?: string; config?: string }): Promise<PruneguardConfig>
```

### Example

```js
import { loadConfig } from "pruneguard";

const config = await loadConfig();
console.log(JSON.stringify(config, null, 2));
```

---

## run

Escape hatch for running arbitrary CLI arguments. Returns raw stdout,
stderr, exit code, and timing.

```ts
function run(args: string[], options?: { cwd?: string }): Promise<CommandResult>
```

### CommandResult

```ts
{
  args: string[];
  cwd?: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
}
```

### Example

```js
import { run } from "pruneguard";

const result = await run(["--format", "json", "--no-cache", "scan"]);
console.log(`Exit: ${result.exitCode}, Duration: ${result.durationMs}ms`);

if (result.exitCode === 0 || result.exitCode === 1) {
  const report = JSON.parse(result.stdout);
  console.log(`Findings: ${report.findings.length}`);
}
```

---

## scanDot

Run a scan and return the Graphviz DOT representation of the module graph.

```ts
function scanDot(options?: ScanOptions): Promise<string>
```

### Example

```js
import { scanDot } from "pruneguard";
import { writeFileSync } from "node:fs";

const dot = await scanDot();
writeFileSync("graph.dot", dot);
// Then: dot -Tsvg -o graph.svg graph.dot
```

---

## binaryPath

Return the absolute path to the resolved native binary.

```ts
function binaryPath(): string
```

### Example

```js
import { binaryPath } from "pruneguard";

console.log(binaryPath());
// => /path/to/node_modules/@pruneguard/cli-darwin-arm64/bin/pruneguard
```

---

## resolutionInfo

Return full resolution diagnostics for the native binary.

```ts
function resolutionInfo(): ResolutionInfo
```

### ResolutionInfo

```ts
{
  binaryPath: string;
  source: "env" | "platform-package" | "dev" | "path";
  platformPackage?: string;
  schemaPath?: string;
  version?: string;
  platform?: string;
}
```

---

## schemaPath

Return the absolute path to the bundled configuration JSON schema file.

```ts
function schemaPath(): string
```

---

## debugResolve

Trace how pruneguard resolves a module specifier from a given source file.

```ts
function debugResolve(options: DebugResolveOptions): Promise<string>
```

### DebugResolveOptions

| Field       | Type     | Description |
|-------------|----------|-------------|
| `cwd`       | `string` | Working directory |
| `config`    | `string` | Config file path |
| `specifier` | `string` | **(required)** The import specifier to resolve |
| `from`      | `string` | **(required)** The source file context |

---

## debugEntrypoints

List all detected entrypoints for the repository.

```ts
function debugEntrypoints(options?: DebugEntrypointsOptions): Promise<string[]>
```

---

## migrateKnip

Convert a knip configuration to pruneguard format.

```ts
function migrateKnip(options?: { cwd?: string; file?: string }): Promise<MigrationOutput>
```

### MigrationOutput

```ts
{
  source: string;
  config: PruneguardConfig;
  warnings: string[];
}
```

---

## migrateDepcruise

Convert a dependency-cruiser configuration to pruneguard format.

```ts
function migrateDepcruise(options?: { cwd?: string; file?: string; node?: boolean }): Promise<MigrationOutput>
```

---

## daemonStatus

Check the status of the pruneguard background daemon.

```ts
function daemonStatus(options?: { cwd?: string }): Promise<DaemonStatusReport>
```

### DaemonStatusReport

```ts
{
  running: boolean;
  pid?: number;
  port?: number;
  version?: string;
  startedAt?: string;
  projectRoot?: string;
}
```

---

## Error handling

All API functions throw `PruneguardExecutionError` on failure. The error
carries a `code` field for programmatic handling.

### Error codes

| Code                             | Meaning |
|----------------------------------|---------|
| `PRUNEGUARD_BINARY_NOT_FOUND`    | Native binary could not be located |
| `PRUNEGUARD_EXECUTION_FAILED`    | Binary exited with an unexpected code |
| `PRUNEGUARD_JSON_PARSE_FAILED`   | Binary output was not valid JSON |

### Error fields

| Field              | Type     | Description |
|--------------------|----------|-------------|
| `code`             | `string` | One of the codes above |
| `message`          | `string` | Human-readable description |
| `exitCode`         | `number` | Process exit code |
| `stdout`           | `string` | Captured stdout |
| `stderr`           | `string` | Captured stderr |
| `binaryPath`       | `string` | Path to the binary that was invoked |
| `args`             | `string[]` | CLI arguments that were passed |
| `resolutionSource` | `string` | How the binary was found |

### Example

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

---

## Shared types

### Finding

```ts
{
  id: string;
  code: string;
  severity: "error" | "warn" | "info";
  confidence: "high" | "medium" | "low";
  category: string;
  subject: string;
  workspace?: string;
  package?: string;
  message: string;
  evidence: Array<Evidence>;
  suggestion?: string;
  ruleName?: string;
  primaryActionKind?: string;
  actionKinds?: string[];
}
```

### Evidence

```ts
{
  kind: string;
  file?: string;
  line?: number;
  description: string;
}
```

### Action

```ts
{
  id: string;
  kind: string;
  targets: string[];
  why: string;
  preconditions: string[];
  steps: Array<{ description: string; file?: string; action?: string }>;
  verification: string[];
  risk: "low" | "medium" | "high";
  confidence: "high" | "medium" | "low";
}
```

### Profile

```ts
type Profile = "production" | "development" | "all";
```
