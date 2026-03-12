import { fileURLToPath } from "node:url";

import { native } from "./native.js";

export type Profile = "production" | "development" | "all";

export type ScanOptions = {
  cwd?: string;
  config?: string;
  paths?: string[];
  profile?: Profile;
};

export type ImpactOptions = {
  cwd?: string;
  config?: string;
  target: string;
  profile?: Profile;
};

export type ExplainOptions = {
  cwd?: string;
  config?: string;
  query: string;
  profile?: Profile;
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
    files: Array<{ path: string; workspace?: string; kind: string }>;
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
  }>;
  stats: {
    durationMs: number;
    filesParsed: number;
    filesCached: number;
    graphNodes: number;
    graphEdges: number;
  };
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
