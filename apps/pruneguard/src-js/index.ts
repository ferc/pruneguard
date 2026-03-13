import { fileURLToPath } from "node:url";

import {
  PruneguardExecutionError,
  binaryPath as resolveBinaryPath,
  resolutionInfo as resolveResolutionInfo,
  run as runBinary,
  type CommandResult,
  type ResolutionInfo,
  type ResolutionSource,
} from "./runtime.js";

export { PruneguardExecutionError, type CommandResult, type ResolutionInfo, type ResolutionSource };

export type Profile = "production" | "development" | "all";

export type ScanOptions = {
  cwd?: string;
  config?: string;
  paths?: string[];
  profile?: Profile;
  changedSince?: string;
  focus?: string;
  noCache?: boolean;
  noBaseline?: boolean;
  requireFullScope?: boolean;
};

export type ImpactOptions = {
  cwd?: string;
  config?: string;
  target: string;
  profile?: Profile;
  focus?: string;
};

export type ExplainOptions = {
  cwd?: string;
  config?: string;
  query: string;
  profile?: Profile;
  focus?: string;
};

export type DebugResolveOptions = {
  cwd?: string;
  config?: string;
  specifier: string;
  from: string;
};

export type DebugEntrypointsOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
};

export type AnalysisReport = {
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
    files: Array<{
      path: string;
      workspace?: string;
      kind: string;
      role?:
        | "source"
        | "test"
        | "story"
        | "fixture"
        | "example"
        | "template"
        | "benchmark"
        | "config"
        | "generated"
        | "buildOutput";
    }>;
    packages: Array<{ name: string; version?: string; workspace: string; path: string }>;
    workspaces: Array<{ name: string; path: string; packageCount: number }>;
  };
  findings: Array<{
    id: string;
    code: string;
    severity: "error" | "warn" | "info";
    confidence: "high" | "medium" | "low";
    category: string;
    subject: string;
    workspace?: string;
    package?: string;
    message: string;
    evidence: Array<{ kind: string; file?: string; line?: number; description: string }>;
    suggestion?: string;
    ruleName?: string;
    primaryActionKind?: string;
    actionKinds?: string[];
    trustNotes?: string[];
    frameworkContext?: string[];
  }>;
  entrypoints: Array<{
    path: string;
    kind: string;
    profile: string;
    workspace?: string;
    source: string;
  }>;
  stats: {
    durationMs: number;
    filesParsed: number;
    filesCached: number;
    filesDiscovered: number;
    filesResolved: number;
    unresolvedSpecifiers: number;
    unresolvedByReason: {
      missingFile: number;
      unsupportedSpecifier: number;
      tsconfigPathMiss: number;
      exportsConditionMiss: number;
      externalized: number;
    };
    resolvedViaExports: number;
    entrypointsDetected: number;
    graphNodes: number;
    graphEdges: number;
    changedFiles: number;
    affectedFiles: number;
    affectedPackages: number;
    affectedEntrypoints: number;
    baselineApplied: boolean;
    baselineProfileMismatch: boolean;
    suppressedFindings: number;
    newFindings: number;
    focusApplied: boolean;
    focusedFiles: number;
    focusedFindings: number;
    fullScopeRequired: boolean;
    partialScope: boolean;
    partialScopeReason?: string;
    confidenceCounts: {
      high: number;
      medium: number;
      low: number;
    };
    parityWarnings: string[];
    cacheHits: number;
    cacheMisses: number;
    cacheEntriesRead: number;
    cacheEntriesWritten: number;
    affectedScopeIncomplete: boolean;
    executionMode?: "oneshot" | "daemon";
    indexWarm?: boolean;
    indexAgeMs?: number;
    reusedGraphNodes?: number;
    reusedGraphEdges?: number;
    watcherLagMs?: number;
  };
};

export type MigrationOutput = {
  source: string;
  config: PruneguardConfig;
  warnings: string[];
};

export type ImpactReport = {
  target: string;
  affectedEntrypoints: string[];
  affectedPackages: string[];
  affectedFiles: string[];
  evidence: Array<{ kind: string; file?: string; line?: number; description: string }>;
  focusFiltered: boolean;
};

export type ExplainReport = {
  query: string;
  matchedNode?: string;
  queryKind: "finding" | "file" | "export";
  proofs: Array<{ node: string; relationship: string; children: ExplainReport["proofs"] }>;
  relatedFindings: AnalysisReport["findings"];
  focusFiltered: boolean;
};

export type PruneguardConfig = Record<string, unknown>;

export type ReviewOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
  baseRef?: string;
  noCache?: boolean;
  noBaseline?: boolean;
  strictTrust?: boolean;
};

export type ReviewReport = {
  baseRef?: string;
  changedFiles: string[];
  newFindings: AnalysisReport["findings"];
  blockingFindings: AnalysisReport["findings"];
  advisoryFindings: AnalysisReport["findings"];
  trust: {
    fullScope: boolean;
    baselineApplied: boolean;
    unresolvedPressure: number;
    confidenceCounts: { high: number; medium: number; low: number };
  };
  recommendations: string[];
  proposedActions?: Array<{
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
  compatibilityWarnings?: string[];
  strictTrustApplied?: boolean;
  recommendedActions?: Array<{ kind: string; description: string; priority: number; command?: string; targets?: string[]; }>;
  executionMode?: "oneshot" | "daemon";
  latencyMs?: number;
};

export type SafeDeleteOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
  targets: string[];
  noCache?: boolean;
};

export type SafeDeleteReport = {
  targets: string[];
  safe: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  needsReview: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  blocked: Array<{ target: string; confidence?: "high" | "medium" | "low"; reasons: string[] }>;
  deletionOrder: string[];
  evidence: Array<{ kind: string; file?: string; line?: number; description: string }>;
};

export type FixPlanOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
  targets: string[];
  noCache?: boolean;
};

export type SuggestRulesOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
  noCache?: boolean;
};

export type SuggestRulesReport = {
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
};

export type FixPlanReport = {
  query: string[];
  matchedFindings: AnalysisReport["findings"];
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
};

export type DaemonStatusReport = {
  running: boolean;
  pid?: number;
  port?: number;
  version?: string;
  startedAt?: string;
  projectRoot?: string;
  indexWarm?: boolean;
  lastUpdateMs?: number;
  graphNodes?: number;
  graphEdges?: number;
  watcherLagMs?: number;
  pendingInvalidations?: number;
  generation?: number;
  uptimeSecs?: number;
};

export type CompatibilityReportOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
};

export type DebugFrameworksOptions = {
  cwd?: string;
  config?: string;
  profile?: Profile;
};

export type CompatibilityReport = {
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
};

export type FrameworkDebugReport = {
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
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function parseJson<T>(result: CommandResult): T {
  try {
    return JSON.parse(result.stdout) as T;
  } catch {
    throw new PruneguardExecutionError(
      "PRUNEGUARD_JSON_PARSE_FAILED",
      `Failed to parse pruneguard JSON output: ${result.stderr || result.stdout.slice(0, 200)}`,
      {
        exitCode: result.exitCode,
        stdout: result.stdout,
        stderr: result.stderr,
        args: result.args,
      },
    );
  }
}

function requireSuccess(result: CommandResult): void {
  if (result.exitCode !== 0) {
    throw new PruneguardExecutionError(
      "PRUNEGUARD_EXECUTION_FAILED",
      `pruneguard exited with code ${result.exitCode}: ${result.stderr}`,
      {
        exitCode: result.exitCode,
        stdout: result.stdout,
        stderr: result.stderr,
        args: result.args,
      },
    );
  }
}

function pushGlobalFlags(
  args: string[],
  options: { config?: string; profile?: Profile; focus?: string },
): void {
  if (options.config) args.push("--config", options.config);
  if (options.profile) args.push("--profile", options.profile);
  if (options.focus) args.push("--focus", options.focus);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export async function scan(options: ScanOptions = {}): Promise<AnalysisReport> {
  const args = ["--format", "json", "--severity", "info"];
  pushGlobalFlags(args, options);
  if (options.changedSince) args.push("--changed-since", options.changedSince);
  if (options.noCache) args.push("--no-cache");
  if (options.noBaseline) args.push("--no-baseline");
  if (options.requireFullScope) args.push("--require-full-scope");
  args.push("scan");
  if (options.paths?.length) args.push(...options.paths);

  const result = await runBinary(args, { cwd: options.cwd });
  // scan: exit 0 = no findings, exit 1 = findings present — both produce valid JSON
  if (result.exitCode !== 0 && result.exitCode !== 1) {
    requireSuccess(result);
  }
  return parseJson<AnalysisReport>(result);
}

export async function scanDot(options: ScanOptions = {}): Promise<string> {
  const args = ["--format", "dot", "--severity", "info"];
  pushGlobalFlags(args, options);
  if (options.changedSince) args.push("--changed-since", options.changedSince);
  if (options.noCache) args.push("--no-cache");
  if (options.noBaseline) args.push("--no-baseline");
  if (options.requireFullScope) args.push("--require-full-scope");
  args.push("scan");
  if (options.paths?.length) args.push(...options.paths);

  const result = await runBinary(args, { cwd: options.cwd });
  if (result.exitCode !== 0 && result.exitCode !== 1) {
    requireSuccess(result);
  }
  return result.stdout;
}

export async function impact(options: ImpactOptions): Promise<ImpactReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  args.push("impact", options.target);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<ImpactReport>(result);
}

export async function explain(options: ExplainOptions): Promise<ExplainReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  args.push("explain", options.query);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<ExplainReport>(result);
}

export async function loadConfig(options?: {
  cwd?: string;
  config?: string;
}): Promise<PruneguardConfig> {
  const args: string[] = [];
  if (options?.config) args.push("--config", options.config);
  args.push("print-config");

  const result = await runBinary(args, { cwd: options?.cwd });
  requireSuccess(result);
  return parseJson<PruneguardConfig>(result);
}

export function schemaPath(): string {
  return fileURLToPath(new URL("../configuration_schema.json", import.meta.url));
}

export function binaryPath(): string {
  return resolveBinaryPath();
}

export function resolutionInfo(): ResolutionInfo {
  return resolveResolutionInfo();
}

export function run(args: string[], options?: { cwd?: string }): Promise<CommandResult> {
  return runBinary(args, options);
}

export async function debugResolve(options: DebugResolveOptions): Promise<string> {
  const args: string[] = [];
  if (options.config) args.push("--config", options.config);
  args.push("debug", "resolve", "--from", options.from, options.specifier);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return result.stdout.trimEnd();
}

export async function debugEntrypoints(
  options: DebugEntrypointsOptions = {},
): Promise<string[]> {
  const args: string[] = [];
  if (options.config) args.push("--config", options.config);
  if (options.profile) args.push("--profile", options.profile);
  args.push("debug", "entrypoints");

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return result.stdout.trimEnd().split("\n").filter(Boolean);
}

export async function review(options: ReviewOptions = {}): Promise<ReviewReport> {
  const args = ["--format", "json", "--severity", "info"];
  pushGlobalFlags(args, options);
  if (options.baseRef) args.push("--changed-since", options.baseRef);
  if (options.noCache) args.push("--no-cache");
  if (options.noBaseline) args.push("--no-baseline");
  if (options.strictTrust) args.push("--strict-trust");
  args.push("review");

  const result = await runBinary(args, { cwd: options.cwd });
  if (result.exitCode !== 0 && result.exitCode !== 1) {
    requireSuccess(result);
  }
  return parseJson<ReviewReport>(result);
}

export async function safeDelete(options: SafeDeleteOptions): Promise<SafeDeleteReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  if (options.noCache) args.push("--no-cache");
  args.push("safe-delete", ...options.targets);

  const result = await runBinary(args, { cwd: options.cwd });
  if (result.exitCode !== 0 && result.exitCode !== 1) {
    requireSuccess(result);
  }
  return parseJson<SafeDeleteReport>(result);
}

export async function fixPlan(options: FixPlanOptions): Promise<FixPlanReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  if (options.noCache) args.push("--no-cache");
  args.push("fix-plan", ...options.targets);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<FixPlanReport>(result);
}

export async function suggestRules(options: SuggestRulesOptions = {}): Promise<SuggestRulesReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  if (options.noCache) args.push("--no-cache");
  args.push("suggest-rules");

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<SuggestRulesReport>(result);
}

export async function migrateKnip(options: {
  cwd?: string;
  file?: string;
} = {}): Promise<MigrationOutput> {
  const args = ["--format", "json", "migrate", "knip"];
  if (options.file) args.push(options.file);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<MigrationOutput>(result);
}

export async function migrateDepcruise(options: {
  cwd?: string;
  file?: string;
  node?: boolean;
} = {}): Promise<MigrationOutput> {
  const args = ["--format", "json", "migrate", "depcruise"];
  if (options.node) args.push("--node");
  if (options.file) args.push(options.file);

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<MigrationOutput>(result);
}

export async function daemonStatus(options?: {
  cwd?: string;
}): Promise<DaemonStatusReport> {
  const args = ["daemon", "status"];

  const result = await runBinary(args, { cwd: options?.cwd });
  if (result.exitCode === 1 && result.stdout.includes("no running daemon")) {
    return { running: false };
  }
  if (result.exitCode !== 0) {
    return { running: false };
  }

  // Parse the text output from daemon status
  const lines = result.stdout.trim().split("\n");
  const report: DaemonStatusReport = { running: true };
  for (const line of lines) {
    const [key, ...rest] = line.split(": ");
    const value = rest.join(": ").trim();
    if (key === "pid") report.pid = parseInt(value, 10);
    else if (key === "port") report.port = parseInt(value, 10);
    else if (key === "version") report.version = value;
    else if (key === "started_at") report.startedAt = value;
  }
  return report;
}

export async function compatibilityReport(options: CompatibilityReportOptions = {}): Promise<CompatibilityReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  args.push("compatibility-report");

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<CompatibilityReport>(result);
}

export async function debugFrameworks(options: DebugFrameworksOptions = {}): Promise<FrameworkDebugReport> {
  const args = ["--format", "json"];
  pushGlobalFlags(args, options);
  args.push("debug", "frameworks");

  const result = await runBinary(args, { cwd: options.cwd });
  requireSuccess(result);
  return parseJson<FrameworkDebugReport>(result);
}
