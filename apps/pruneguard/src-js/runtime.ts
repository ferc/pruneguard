import { execFileSync, spawn } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));

export type ResolutionSource = "env" | "platform-package" | "dev" | "path";

export type ResolutionInfo = {
  binaryPath: string;
  source: ResolutionSource;
  platformPackage?: string;
  schemaPath?: string;
  version?: string;
  platform?: string;
};

export type CommandResult = {
  args: string[];
  cwd?: string;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
};

export class PruneguardExecutionError extends Error {
  code: "PRUNEGUARD_BINARY_NOT_FOUND" | "PRUNEGUARD_EXECUTION_FAILED" | "PRUNEGUARD_JSON_PARSE_FAILED";
  exitCode?: number;
  stdout?: string;
  stderr?: string;
  binaryPath?: string;
  args?: string[];
  resolutionSource?: ResolutionSource;

  constructor(
    code: PruneguardExecutionError["code"],
    message: string,
    details?: {
      exitCode?: number;
      stdout?: string;
      stderr?: string;
      binaryPath?: string;
      args?: string[];
      resolutionSource?: ResolutionSource;
    },
  ) {
    super(message);
    this.name = "PruneguardExecutionError";
    this.code = code;
    this.exitCode = details?.exitCode;
    this.stdout = details?.stdout;
    this.stderr = details?.stderr;
    this.binaryPath = details?.binaryPath;
    this.args = details?.args;
    this.resolutionSource = details?.resolutionSource;
  }
}

const PLATFORM_PACKAGES: Record<string, string[]> = {
  "darwin-arm64": ["@pruneguard/cli-darwin-arm64"],
  "darwin-x64": ["@pruneguard/cli-darwin-x64"],
  "linux-arm64": ["@pruneguard/cli-linux-arm64-gnu", "@pruneguard/cli-linux-arm64-musl"],
  "linux-x64": ["@pruneguard/cli-linux-x64-gnu", "@pruneguard/cli-linux-x64-musl"],
  "win32-arm64": ["@pruneguard/cli-win32-arm64-msvc"],
  "win32-x64": ["@pruneguard/cli-win32-x64-msvc"],
};

function exeName(): string {
  return process.platform === "win32" ? "pruneguard.exe" : "pruneguard";
}

function findPlatformBinary(): { path: string; packageName: string } | undefined {
  const key = `${process.platform}-${process.arch === "arm64" ? "arm64" : "x64"}`;
  const candidates = PLATFORM_PACKAGES[key];
  if (!candidates) return undefined;

  for (const pkg of candidates) {
    try {
      const pkgJsonPath = require.resolve(`${pkg}/package.json`);
      const binPath = join(dirname(pkgJsonPath), "bin", exeName());
      if (existsSync(binPath)) return { path: binPath, packageName: pkg };
    } catch {
      continue;
    }
  }
  return undefined;
}

function findDevBinary(): string | undefined {
  const candidates = [
    join(__dirname, "..", "..", "..", "target", "release", exeName()),
    join(__dirname, "..", "..", "..", "target", "debug", exeName()),
  ];
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }
  return undefined;
}

function findPathBinary(): string | undefined {
  try {
    const cmd = process.platform === "win32" ? "where" : "which";
    const result = execFileSync(cmd, [exeName()], {
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
    });
    const binPath = result.trim().split("\n")[0];
    if (binPath) return binPath;
  } catch {
    // not found on PATH
  }
  return undefined;
}

let cachedBinaryPath: string | undefined;
let cachedResolutionSource: ResolutionSource | undefined;
let cachedPlatformPackage: string | undefined;

function validateExecutable(binPath: string, source: ResolutionSource): void {
  if (!existsSync(binPath)) {
    throw new PruneguardExecutionError(
      "PRUNEGUARD_BINARY_NOT_FOUND",
      `[${source}] Binary does not exist: ${binPath}. Run "pruneguard debug runtime" for diagnostics.`,
      { binaryPath: binPath, resolutionSource: source },
    );
  }
  if (process.platform !== "win32") {
    try {
      accessSync(binPath, constants.X_OK);
    } catch {
      throw new PruneguardExecutionError(
        "PRUNEGUARD_BINARY_NOT_FOUND",
        `[${source}] Binary exists but is not executable: ${binPath}. Try: chmod +x "${binPath}"`,
        { binaryPath: binPath, resolutionSource: source },
      );
    }
  }
}

export function binaryPath(options?: { allowPathFallback?: boolean }): string {
  if (cachedBinaryPath) return cachedBinaryPath;

  // 1. Environment variable
  const envPath = process.env.PRUNEGUARD_BINARY;
  if (envPath) {
    validateExecutable(envPath, "env");
    cachedBinaryPath = envPath;
    cachedResolutionSource = "env";
    return envPath;
  }

  // 2. Installed platform package
  const platformBin = findPlatformBinary();
  if (platformBin) {
    validateExecutable(platformBin.path, "platform-package");
    cachedBinaryPath = platformBin.path;
    cachedResolutionSource = "platform-package";
    cachedPlatformPackage = platformBin.packageName;
    return platformBin.path;
  }

  // 3. Development fallback (cargo-built binary)
  const devBin = findDevBinary();
  if (devBin) {
    validateExecutable(devBin, "dev");
    cachedBinaryPath = devBin;
    cachedResolutionSource = "dev";
    return devBin;
  }

  // 4. PATH fallback (disabled by default)
  if (options?.allowPathFallback) {
    const pathBin = findPathBinary();
    if (pathBin) {
      validateExecutable(pathBin, "path");
      cachedBinaryPath = pathBin;
      cachedResolutionSource = "path";
      return pathBin;
    }
  }

  const key = `${process.platform}-${process.arch === "arm64" ? "arm64" : "x64"}`;
  const expectedPkgs = PLATFORM_PACKAGES[key];
  const tried = ["env(PRUNEGUARD_BINARY)", "platform-package", "dev(cargo build)"];
  if (options?.allowPathFallback) tried.push("PATH");
  const pkgHint = expectedPkgs?.length
    ? `\n  Expected platform package: ${expectedPkgs.join(" or ")}`
    : "";
  const platform = `${process.platform}-${process.arch}`;

  throw new PruneguardExecutionError(
    "PRUNEGUARD_BINARY_NOT_FOUND",
    `Could not find the pruneguard binary for ${platform}.\n  Tried: ${tried.join(", ")}${pkgHint}\n  Fix: npm install pruneguard (or set PRUNEGUARD_BINARY)\n  Debug: npx pruneguard debug runtime`,
  );
}

export function resolutionInfo(): ResolutionInfo {
  const bin = binaryPath();
  return {
    binaryPath: bin,
    source: cachedResolutionSource!,
    ...(cachedPlatformPackage ? { platformPackage: cachedPlatformPackage } : {}),
    schemaPath: join(dirname(fileURLToPath(import.meta.url)), "..", "configuration_schema.json"),
    platform: `${process.platform}-${process.arch}`,
  };
}

export function run(args: string[], options?: { cwd?: string }): Promise<CommandResult> {
  const binary = binaryPath();
  const start = performance.now();

  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, {
      cwd: options?.cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });

    child.on("error", (err) => {
      reject(
        new PruneguardExecutionError("PRUNEGUARD_EXECUTION_FAILED", `Failed to spawn pruneguard: ${err.message}`, {
          binaryPath: binary,
          args,
          resolutionSource: cachedResolutionSource,
        }),
      );
    });

    child.on("close", (exitCode) => {
      resolve({
        args,
        cwd: options?.cwd,
        exitCode: exitCode ?? 1,
        stdout,
        stderr,
        durationMs: Math.round(performance.now() - start),
      });
    });
  });
}
