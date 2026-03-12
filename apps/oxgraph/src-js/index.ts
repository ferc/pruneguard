import { fileURLToPath } from "node:url";

import { native } from "./native.js";

export type Profile = "production" | "development" | "all";

export type ScanOptions = {
  cwd?: string;
  config?: string;
  paths?: string[];
  profile?: Profile;
  changedSince?: string;
  focus?: string;
  noCache?: boolean;
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
  config: OxgraphConfig;
  warnings: string[];
};

export type ImpactReport = {
  target: string;
  affectedEntrypoints: string[];
  affectedPackages: string[];
  affectedFiles: string[];
  evidence: Array<{ kind: string; file?: string; line?: number; description: string }>;
};

export type ExplainReport = {
  query: string;
  matchedNode?: string;
  proofs: Array<{ node: string; relationship: string; children: ExplainReport["proofs"] }>;
  relatedFindings: AnalysisReport["findings"];
};

export type OxgraphConfig = Record<string, unknown>;

export function scan(options: ScanOptions = {}): AnalysisReport {
  return JSON.parse(native.scan_json(options)) as AnalysisReport;
}

/** @experimental */
export function scanDot(options: ScanOptions = {}): string {
  return native.scan_dot_text(options);
}

export function impact(options: ImpactOptions): ImpactReport {
  return JSON.parse(native.impact_json(options)) as ImpactReport;
}

export function explain(options: ExplainOptions): ExplainReport {
  return JSON.parse(native.explain_json(options)) as ExplainReport;
}

export function loadConfig(cwd = process.cwd(), config?: string): OxgraphConfig {
  return JSON.parse(native.load_config_json(cwd, config)) as OxgraphConfig;
}

export function schemaPath(): string {
  return fileURLToPath(new URL("../configuration_schema.json", import.meta.url));
}

export function debugResolve(options: DebugResolveOptions): string {
  return native.debug_resolve_text(options);
}

export function debugEntrypoints(
  options: DebugEntrypointsOptions = {},
): string[] {
  return JSON.parse(native.debug_entrypoints_json(options)) as string[];
}

/** @experimental */
export function migrateKnip(options: {
  cwd?: string;
  file?: string;
} = {}): MigrationOutput {
  return JSON.parse(native.migrate_knip_json(options)) as MigrationOutput;
}

/** @experimental */
export function migrateDepcruise(options: {
  cwd?: string;
  file?: string;
  node?: boolean;
} = {}): MigrationOutput {
  return JSON.parse(native.migrate_depcruise_json(options)) as MigrationOutput;
}
