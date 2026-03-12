#!/usr/bin/env node

import { spawn } from "node:child_process";

import { PruneguardExecutionError, binaryPath } from "./runtime.js";

try {
  const binary = binaryPath({ allowPathFallback: true });
  const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });

  for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
    process.on(sig, () => child.kill(sig));
  }

  child.on("close", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exitCode = code ?? 1;
    }
  });
} catch (err) {
  console.error(err instanceof PruneguardExecutionError ? err.message : String(err));
  process.exitCode = 2;
}
