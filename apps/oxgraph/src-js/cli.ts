#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

import {
  debugEntrypoints,
  debugResolve,
  explain,
  impact,
  loadConfig,
  scan,
} from "./index.js";

type OutputFormat = "text" | "json" | "sarif" | "dot";
type Profile = "production" | "development" | "all";
type Severity = "error" | "warn" | "info";

type Parsed = {
  config?: string;
  format: OutputFormat;
  profile: Profile;
  severity: Severity;
  maxFindings?: number;
  command: string[];
};

function main(): void {
  try {
    const parsed = parseArgs(process.argv.slice(2));
    run(parsed);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(message);
    process.exitCode = 2;
  }
}

function run(parsed: Parsed): void {
  const [command = "scan", ...rest] = parsed.command;
  switch (command) {
    case "scan": {
      const report = scan({
        cwd: process.cwd(),
        config: parsed.config,
        paths: rest,
        profile: parsed.profile,
      });
      const findings = filterFindings(report.findings, parsed.severity, parsed.maxFindings);
      render(
        {
          ...report,
          findings,
          summary: {
            ...report.summary,
            totalFindings: findings.length,
            errors: findings.filter((finding) => finding.severity === "error").length,
            warnings: findings.filter((finding) => finding.severity === "warn").length,
            infos: findings.filter((finding) => finding.severity === "info").length,
          },
        },
        parsed.format,
      );
      process.exitCode = findings.length > 0 ? 1 : 0;
      return;
    }
    case "impact": {
      const target = rest[0];
      if (!target) {
        throw new Error("impact requires a target");
      }
      render(
        impact({ cwd: process.cwd(), config: parsed.config, target, profile: parsed.profile }),
        parsed.format,
      );
      return;
    }
    case "explain": {
      const query = rest[0];
      if (!query) {
        throw new Error("explain requires a query");
      }
      render(
        explain({ cwd: process.cwd(), config: parsed.config, query, profile: parsed.profile }),
        parsed.format,
      );
      return;
    }
    case "init": {
      const output = path.join(process.cwd(), "oxgraph.json");
      fs.writeFileSync(output, JSON.stringify(defaultConfig(), null, 2));
      console.error("Created oxgraph.json");
      return;
    }
    case "print-config": {
      console.log(JSON.stringify(loadConfig(process.cwd(), parsed.config), null, 2));
      return;
    }
    case "debug": {
      const [subcommand, ...debugArgs] = rest;
      if (subcommand === "resolve") {
        const fromIndex = debugArgs.indexOf("--from");
        if (fromIndex === -1 || !debugArgs[0] || !debugArgs[fromIndex + 1]) {
          throw new Error("debug resolve <specifier> --from <file>");
        }
        const specifier = debugArgs[0];
        const from = debugArgs[fromIndex + 1];
        console.log(
          debugResolve({ cwd: process.cwd(), config: parsed.config, specifier, from }),
        );
        return;
      }
      if (subcommand === "entrypoints") {
        for (const entrypoint of debugEntrypoints({
          cwd: process.cwd(),
          config: parsed.config,
          profile: parsed.profile,
        })) {
          console.log(entrypoint);
        }
        return;
      }
      throw new Error("debug requires `resolve` or `entrypoints`");
    }
    case "migrate":
      throw new Error("migration commands are not implemented yet");
    case "--help":
    case "-h":
    case "help":
      printHelp();
      return;
    default: {
      const report = scan({
        cwd: process.cwd(),
        config: parsed.config,
        paths: [command, ...rest],
        profile: parsed.profile,
      });
      render(report, parsed.format);
      process.exitCode = report.findings.length > 0 ? 1 : 0;
    }
  }
}

function parseArgs(argv: string[]): Parsed {
  let config: string | undefined;
  let format: OutputFormat = "text";
  let profile: Profile = "all";
  let severity: Severity = "warn";
  let maxFindings: number | undefined;
  const command: string[] = [];

  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "-c" || value === "--config") {
      config = argv[index + 1];
      index += 1;
      continue;
    }
    if (value === "--format") {
      format = (argv[index + 1] as OutputFormat | undefined) ?? "text";
      index += 1;
      continue;
    }
    if (value === "--profile") {
      profile = (argv[index + 1] as Profile | undefined) ?? "all";
      index += 1;
      continue;
    }
    if (value === "--severity") {
      severity = (argv[index + 1] as Severity | undefined) ?? "warn";
      index += 1;
      continue;
    }
    if (value === "--max-findings") {
      maxFindings = Number(argv[index + 1] ?? "0");
      index += 1;
      continue;
    }
    if (value === "--help" || value === "-h") {
      return { config, format, profile, severity, maxFindings, command: ["help"] };
    }
    command.push(value);
  }

  return { config, format, profile, severity, maxFindings, command };
}

function filterFindings<T extends { severity: Severity }>(
  findings: T[],
  threshold: Severity,
  maxFindings?: number,
): T[] {
  const filtered = findings.filter((finding) => {
    if (threshold === "info") return true;
    if (threshold === "warn") return finding.severity !== "info";
    return finding.severity === "error";
  });
  return typeof maxFindings === "number" ? filtered.slice(0, maxFindings) : filtered;
}

function render(report: unknown, format: OutputFormat): void {
  if (format === "json") {
    console.log(JSON.stringify(report, null, 2));
    return;
  }

  if (format === "sarif") {
    const findings = (report as { findings?: Array<Record<string, unknown>> }).findings ?? [];
    console.log(
      JSON.stringify(
        {
          $schema: "https://json.schemastore.org/sarif-2.1.0.json",
          version: "2.1.0",
          runs: [
            {
              tool: { driver: { name: "oxgraph" } },
              results: findings.map((finding) => ({
                ruleId: finding.code ?? "oxgraph",
                level:
                  finding.severity === "error"
                    ? "error"
                    : finding.severity === "info"
                      ? "note"
                      : "warning",
                message: { text: finding.message ?? "" },
                locations: [
                  {
                    physicalLocation: {
                      artifactLocation: { uri: finding.subject ?? "" },
                    },
                  },
                ],
              })),
            },
          ],
        },
        null,
        2,
      ),
    );
    return;
  }

  if (format === "dot") {
    throw new Error("dot output is not implemented yet");
  }

  const asRecord = report as Record<string, unknown>;
  if ("summary" in asRecord && "findings" in asRecord) {
    const summary = asRecord.summary as Record<string, number>;
    const findings = (asRecord.findings as Array<Record<string, unknown>>) ?? [];
    const entrypoints = ((asRecord.entrypoints as unknown[]) ?? []).length;
    console.log(`files: ${summary.totalFiles ?? 0}`);
    console.log(`packages: ${summary.totalPackages ?? 0}`);
    console.log(`entrypoints: ${entrypoints}`);
    console.log(`findings: ${findings.length}`);
    if (findings.length > 0) {
      console.log("");
      for (const finding of findings) {
        console.log(
          `[${finding.severity ?? "warn"}] ${finding.code ?? "finding"} ${finding.subject ?? ""}`,
        );
        console.log(`  ${finding.message ?? ""}`);
      }
    }
    return;
  }

  console.log(JSON.stringify(report, null, 2));
}

function defaultConfig(): Record<string, unknown> {
  return {
    $schema: "./node_modules/oxgraph/configuration_schema.json",
    workspaces: {
      packageManager: "auto",
      roots: ["apps/*", "packages/*"],
    },
    entrypoints: {
      auto: true,
      includeTests: false,
      includeStories: false,
    },
    analysis: {
      unusedExports: "warn",
      unusedFiles: "warn",
      unusedDependencies: "warn",
      unusedPackages: "warn",
      cycles: "warn",
      boundaries: "warn",
      ownership: "warn",
      impact: "warn",
    },
  };
}

function printHelp(): void {
  console.log("oxgraph - Repo truth engine for JS/TS monorepos");
  console.log("");
  console.log("Usage:");
  console.log("  oxgraph [paths...]");
  console.log("  oxgraph scan [paths...]");
  console.log("  oxgraph impact <target>");
  console.log("  oxgraph explain <query>");
  console.log("  oxgraph init");
  console.log("  oxgraph print-config");
  console.log("  oxgraph debug resolve <specifier> --from <file>");
  console.log("  oxgraph debug entrypoints");
  console.log("  oxgraph migrate knip [file]");
  console.log("  oxgraph migrate depcruise [file]");
}

main();
