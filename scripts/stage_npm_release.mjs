#!/usr/bin/env node

/**
 * Stage npm packages for publishing.
 *
 * Copies npm/pruneguard and npm/cli-* into .release/npm/, rewrites workspace:*
 * references to concrete semver, and injects shared metadata into platform
 * packages.
 *
 * Usage:
 *   node scripts/stage_npm_release.mjs
 *   node scripts/stage_npm_release.mjs --version 0.3.0   # override version
 *   node scripts/stage_npm_release.mjs --verify           # validate staged output
 */

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const NPM_SRC = join(ROOT, "npm");
const RELEASE_DIR = join(ROOT, ".release", "npm");

// Build artifacts that must never appear in staged output
const BANNED_ENTRIES = [".tsbuildinfo", ".turbo", "node_modules", ".DS_Store"];

// ---------------------------------------------------------------------------
// Parse args
// ---------------------------------------------------------------------------

let versionOverride;
let verifyOnly = false;
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--version" && args[i + 1]) {
    versionOverride = args[++i];
  } else if (args[i] === "--verify") {
    verifyOnly = true;
  }
}

// ---------------------------------------------------------------------------
// Read source version
// ---------------------------------------------------------------------------

const rootPkg = JSON.parse(readFileSync(join(NPM_SRC, "pruneguard", "package.json"), "utf8"));
const version = versionOverride ?? rootPkg.version;

// ---------------------------------------------------------------------------
// --verify mode: validate staged output without re-staging
// ---------------------------------------------------------------------------

if (verifyOnly) {
  console.log(`Verifying staged packages in ${RELEASE_DIR}`);
  const errors = validateStagedOutput(version);
  if (errors.length > 0) {
    for (const err of errors) console.error(`ERROR: ${err}`);
    process.exit(1);
  }
  console.log("Verification passed.");
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Stage mode (default)
// ---------------------------------------------------------------------------

console.log(`Staging pruneguard@${version} into ${RELEASE_DIR}`);

// ---------------------------------------------------------------------------
// Shared metadata injected into every platform package
// ---------------------------------------------------------------------------

const sharedMeta = {
  license: rootPkg.license,
  repository: {
    type: "git",
    url: rootPkg.repository.url,
  },
  homepage: rootPkg.homepage,
  bugs: rootPkg.bugs,
};

// Publish metadata injected into platform packages
const publishMeta = {
  description: "pruneguard platform binary",
  keywords: rootPkg.keywords ?? [],
  engines: { node: ">=18" },
};

// ---------------------------------------------------------------------------
// Clean previous staging
// ---------------------------------------------------------------------------

if (existsSync(RELEASE_DIR)) {
  rmSync(RELEASE_DIR, { recursive: true });
}
mkdirSync(RELEASE_DIR, { recursive: true });

// ---------------------------------------------------------------------------
// Stage root package (npm/pruneguard -> .release/npm/pruneguard)
// ---------------------------------------------------------------------------

const rootSrc = join(NPM_SRC, "pruneguard");
const rootDst = join(RELEASE_DIR, "pruneguard");

cpSync(rootSrc, rootDst, { recursive: true });

// Clean build artifacts from staged root package
cleanBuildArtifacts(rootDst);

// Rewrite workspace:* to concrete version
const stagedRootPkg = JSON.parse(readFileSync(join(rootDst, "package.json"), "utf8"));
stagedRootPkg.version = version;

for (const field of ["dependencies", "optionalDependencies", "peerDependencies", "devDependencies"]) {
  if (!stagedRootPkg[field]) continue;
  for (const [dep, ver] of Object.entries(stagedRootPkg[field])) {
    if (ver.startsWith("workspace:")) {
      stagedRootPkg[field][dep] = version;
    }
  }
}

writeFileSync(join(rootDst, "package.json"), JSON.stringify(stagedRootPkg, null, 2) + "\n");
console.log(`  ✓ pruneguard — workspace:* → ${version}`);

// ---------------------------------------------------------------------------
// Stage platform packages (npm/cli-* -> .release/npm/cli-*)
// ---------------------------------------------------------------------------

const platformDirs = readdirSync(NPM_SRC).filter((d) => d.startsWith("cli-"));

for (const dir of platformDirs) {
  const src = join(NPM_SRC, dir);
  const dst = join(RELEASE_DIR, dir);

  cpSync(src, dst, { recursive: true });

  // Clean build artifacts from staged platform package
  cleanBuildArtifacts(dst);

  const pkgPath = join(dst, "package.json");
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));

  // Sync version
  pkg.version = version;

  // Inject shared metadata
  Object.assign(pkg, sharedMeta);

  // Inject publish metadata (description only if not already set with
  // platform-specific wording, keywords and engines always overwritten)
  if (!pkg.description) {
    pkg.description = publishMeta.description;
  }
  pkg.keywords = publishMeta.keywords;
  pkg.engines = publishMeta.engines;

  // Set repository.directory for this specific package
  pkg.repository = {
    ...pkg.repository,
    directory: `npm/${dir}`,
  };

  writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
  console.log(`  ✓ ${pkg.name}@${version}`);
}

// ---------------------------------------------------------------------------
// Post-staging validation
// ---------------------------------------------------------------------------

console.log(`\nStaged ${1 + platformDirs.length} packages into ${RELEASE_DIR}`);

const errors = validateStagedOutput(version);
if (errors.length > 0) {
  for (const err of errors) console.error(`ERROR: ${err}`);
  process.exit(1);
}

console.log("Staged output validated successfully.");

// ===========================================================================
// Helper functions
// ===========================================================================

/**
 * Recursively remove banned build artifacts from a directory.
 */
function cleanBuildArtifacts(dir) {
  for (const entry of readdirSync(dir)) {
    const fullPath = join(dir, entry);
    if (BANNED_ENTRIES.includes(entry)) {
      rmSync(fullPath, { recursive: true, force: true });
      continue;
    }
    try {
      if (statSync(fullPath).isDirectory()) {
        cleanBuildArtifacts(fullPath);
      }
    } catch {
      // ignore stat errors (e.g. broken symlinks)
    }
  }
}

/**
 * Walk a directory tree and return all file/directory names.
 */
function walkEntryNames(dir) {
  const names = [];
  for (const entry of readdirSync(dir)) {
    names.push(entry);
    const fullPath = join(dir, entry);
    try {
      if (statSync(fullPath).isDirectory()) {
        names.push(...walkEntryNames(fullPath));
      }
    } catch {
      // ignore
    }
  }
  return names;
}

/**
 * Validate the staged output in RELEASE_DIR. Returns an array of error
 * strings (empty = pass).
 */
function validateStagedOutput(ver) {
  const errs = [];

  if (!existsSync(RELEASE_DIR)) {
    errs.push(`Release directory does not exist: ${RELEASE_DIR}`);
    return errs;
  }

  // Discover staged platform dirs
  const stagedPlatformDirs = readdirSync(RELEASE_DIR).filter((d) => d.startsWith("cli-"));

  // --- Validate each platform package ---
  const requiredPlatformFields = ["name", "version", "os", "cpu", "description", "license", "repository"];

  for (const dir of stagedPlatformDirs) {
    const pkgPath = join(RELEASE_DIR, dir, "package.json");
    if (!existsSync(pkgPath)) {
      errs.push(`Missing package.json in staged platform package: ${dir}`);
      continue;
    }

    const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));

    // Required fields
    for (const field of requiredPlatformFields) {
      if (pkg[field] === undefined || pkg[field] === null) {
        errs.push(`${dir}: missing required field "${field}"`);
      }
    }

    // Must have a bin field (via files referencing bin/)
    if (!pkg.files || !pkg.files.some((f) => f.startsWith("bin/"))) {
      errs.push(`${dir}: no bin entry in "files" array`);
    }

    // Binaries must ONLY live in .release/npm/cli-*/bin/
    const dirPath = join(RELEASE_DIR, dir);
    for (const entry of readdirSync(dirPath)) {
      if (entry === "package.json" || entry === "bin") continue;
      const fullPath = join(dirPath, entry);
      // Any other file that looks like a binary is a problem
      if (entry === "pruneguard" || entry === "pruneguard.exe") {
        errs.push(`${dir}: binary "${entry}" found outside bin/ directory`);
      }
    }
  }

  // --- Validate root package ---
  const rootPkgPath = join(RELEASE_DIR, "pruneguard", "package.json");
  if (!existsSync(rootPkgPath)) {
    errs.push("Missing staged root package.json");
    return errs;
  }

  const stagedRoot = JSON.parse(readFileSync(rootPkgPath, "utf8"));

  // Verify no workspace: references remain
  for (const field of ["dependencies", "optionalDependencies", "peerDependencies", "devDependencies"]) {
    for (const [dep, v] of Object.entries(stagedRoot[field] || {})) {
      if (v.startsWith("workspace:")) {
        errs.push(`Root package still has workspace: reference: ${dep}@${v}`);
      }
    }
  }

  // Verify root bin entry points to an existing file
  if (stagedRoot.bin) {
    for (const [cmd, binPath] of Object.entries(stagedRoot.bin)) {
      const resolvedBin = join(RELEASE_DIR, "pruneguard", binPath);
      if (!existsSync(resolvedBin)) {
        errs.push(`Root package bin "${cmd}" points to non-existent file: ${binPath}`);
      }
    }
  }

  // Verify optionalDependencies lists exactly the staged platform packages
  const optDeps = Object.keys(stagedRoot.optionalDependencies || {}).sort();
  const expectedOptDeps = stagedPlatformDirs
    .map((d) => {
      const pkgPath = join(RELEASE_DIR, d, "package.json");
      return JSON.parse(readFileSync(pkgPath, "utf8")).name;
    })
    .sort();

  if (JSON.stringify(optDeps) !== JSON.stringify(expectedOptDeps)) {
    errs.push(
      `Root optionalDependencies mismatch.\n` +
        `  Expected: ${expectedOptDeps.join(", ")}\n` +
        `  Got:      ${optDeps.join(", ")}`,
    );
  }

  // --- Check for banned artifacts across ALL staged packages ---
  const allEntryNames = walkEntryNames(RELEASE_DIR);
  for (const banned of BANNED_ENTRIES) {
    if (allEntryNames.includes(banned)) {
      errs.push(`Banned artifact "${banned}" found in staged output`);
    }
  }

  return errs;
}
