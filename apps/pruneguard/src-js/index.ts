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
