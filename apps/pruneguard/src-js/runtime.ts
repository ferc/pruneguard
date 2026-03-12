import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));

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

  constructor(
    code: PruneguardExecutionError["code"],
    message: string,
    details?: {
      exitCode?: number;
      stdout?: string;
      stderr?: string;
      binaryPath?: string;
      args?: string[];
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

function findPlatformBinary(): string | undefined {
  const key = `${process.platform}-${process.arch === "arm64" ? "arm64" : "x64"}`;
  const candidates = PLATFORM_PACKAGES[key];
  if (!candidates) return undefined;

  for (const pkg of candidates) {
    try {
      const pkgJsonPath = require.resolve(`${pkg}/package.json`);
      const binPath = join(dirname(pkgJsonPath), "bin", exeName());
      if (existsSync(binPath)) return binPath;
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

export function binaryPath(options?: { allowPathFallback?: boolean }): string {
  if (cachedBinaryPath) return cachedBinaryPath;

  // 1. Environment variable
  const envPath = process.env.PRUNEGUARD_BINARY;
  if (envPath) {
    if (!existsSync(envPath)) {
      throw new PruneguardExecutionError(
        "PRUNEGUARD_BINARY_NOT_FOUND",
        `PRUNEGUARD_BINARY points to ${envPath} but the file does not exist`,
        { binaryPath: envPath },
      );
    }
    cachedBinaryPath = envPath;
    return envPath;
  }

  // 2. Installed platform package
  const platformBin = findPlatformBinary();
  if (platformBin) {
    cachedBinaryPath = platformBin;
    return platformBin;
  }

  // 3. Development fallback (cargo-built binary)
  const devBin = findDevBinary();
  if (devBin) {
    cachedBinaryPath = devBin;
    return devBin;
  }

  // 4. PATH fallback (disabled by default)
  if (options?.allowPathFallback) {
    const pathBin = findPathBinary();
    if (pathBin) {
      cachedBinaryPath = pathBin;
      return pathBin;
    }
  }

  throw new PruneguardExecutionError(
    "PRUNEGUARD_BINARY_NOT_FOUND",
    "Could not find the pruneguard binary. Install a platform-specific package or set PRUNEGUARD_BINARY.",
  );
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
