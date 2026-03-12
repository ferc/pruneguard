#!/usr/bin/env node

/**
 * Stage npm packages for publishing.
 *
 * Copies npm/pruneguard and npm/cli-* into .release/npm/, rewrites workspace:*
 * references to concrete semver, and injects shared metadata into platform
 * packages.
 *
 * .release/npm/ is the ONLY source of truth for npm pack and publish.
 * The repo-local npm/ packages exist only for local development (pnpm workspace).
 *
 * Usage:
 *   node scripts/stage_npm_release.mjs
 *   node scripts/stage_npm_release.mjs --version 0.3.0   # override version
 *   node scripts/stage_npm_release.mjs --verify           # validate staged output
 */

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const NPM_SRC = join(ROOT, "npm");
const RELEASE_DIR = join(ROOT, ".release", "npm");

// Build artifacts that must never appear in staged output
const BANNED_ENTRIES = [".tsbuildinfo", ".turbo", "node_modules", ".DS_Store", ".gitkeep"];

// Patterns that must never appear in staged package.json values
const BANNED_PATTERNS = [
  /workspace:/,         // pnpm workspace protocol
  /file:/,              // local file references
  /link:/,              // pnpm link protocol
  /\.\.\/\.\.\//,       // relative parent paths (../../)
  /\/Users\//,          // absolute macOS paths
  /\/home\//,           // absolute Linux paths
  /[A-Z]:\\/,           // absolute Windows paths
];

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
  const errors = validateStagedOutput(version, { strict: true });
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
console.log(`  staged pruneguard — workspace:* -> ${version}`);

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
  console.log(`  staged ${pkg.name}@${version}`);
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
 * Scan a package.json object for banned patterns in string values.
 * Returns an array of error strings.
 */
function checkForBannedPatterns(pkg, label) {
  const errs = [];
  const stringFields = [
    "dependencies", "optionalDependencies", "peerDependencies", "devDependencies",
    "bin", "main", "module", "types",
  ];

  // Check dependency-like fields
  for (const field of stringFields) {
    const val = pkg[field];
    if (!val) continue;

    if (typeof val === "string") {
      for (const pattern of BANNED_PATTERNS) {
        if (pattern.test(val)) {
          errs.push(`${label}: field "${field}" contains banned pattern ${pattern}: "${val}"`);
        }
      }
    } else if (typeof val === "object") {
      for (const [key, v] of Object.entries(val)) {
        if (typeof v !== "string") continue;
        for (const pattern of BANNED_PATTERNS) {
          if (pattern.test(v)) {
            errs.push(`${label}: ${field}.${key} contains banned pattern ${pattern}: "${v}"`);
          }
        }
      }
    }
  }

  // Check exports field recursively
  if (pkg.exports) {
    const checkExports = (obj, path) => {
      if (typeof obj === "string") {
        for (const pattern of BANNED_PATTERNS) {
          if (pattern.test(obj)) {
            errs.push(`${label}: exports${path} contains banned pattern ${pattern}: "${obj}"`);
          }
        }
      } else if (typeof obj === "object" && obj !== null) {
        for (const [k, v] of Object.entries(obj)) {
          checkExports(v, `${path}.${k}`);
        }
      }
    };
    checkExports(pkg.exports, "");
  }

  return errs;
}

/**
 * Validate the staged output in RELEASE_DIR. Returns an array of error
 * strings (empty = pass).
 *
 * @param {string} ver - Expected version string.
 * @param {object} [options]
 * @param {boolean} [options.strict=false] - When true, also verify that build
 *   artifacts (dist/, schemas) exist in staged output. Used in --verify mode
 *   after all build artifacts have been copied into .release/npm/.
 */
function validateStagedOutput(ver, options = {}) {
  const strict = options.strict ?? false;
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

    // Version must match
    if (pkg.version !== ver) {
      errs.push(`${dir}: version "${pkg.version}" does not match expected "${ver}"`);
    }

    // Must have a bin entry in files array
    if (!pkg.files || !pkg.files.some((f) => f.startsWith("bin/"))) {
      errs.push(`${dir}: no bin entry in "files" array`);
    }

    // Binaries must ONLY live in .release/npm/cli-*/bin/
    const dirPath = join(RELEASE_DIR, dir);
    for (const entry of readdirSync(dirPath)) {
      if (entry === "package.json" || entry === "bin") continue;
      // Any other file that looks like a binary is a problem
      if (entry === "pruneguard" || entry === "pruneguard.exe") {
        errs.push(`${dir}: binary "${entry}" found outside bin/ directory`);
      }
    }

    // Check for banned patterns in package.json
    errs.push(...checkForBannedPatterns(pkg, dir));

    // Verify no workspace: references
    for (const field of ["dependencies", "optionalDependencies", "peerDependencies", "devDependencies"]) {
      for (const [dep, v] of Object.entries(pkg[field] || {})) {
        if (typeof v === "string" && v.startsWith("workspace:")) {
          errs.push(`${dir}: has workspace: reference: ${dep}@${v}`);
        }
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

  // Version must match
  if (stagedRoot.version !== ver) {
    errs.push(`Root package version "${stagedRoot.version}" does not match expected "${ver}"`);
  }

  // Verify no workspace: references remain
  for (const field of ["dependencies", "optionalDependencies", "peerDependencies", "devDependencies"]) {
    for (const [dep, v] of Object.entries(stagedRoot[field] || {})) {
      if (typeof v === "string" && v.startsWith("workspace:")) {
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

  // Verify "files" entries reference existing files/dirs (strict mode only,
  // since build artifacts like dist/ may not exist until after build-js)
  if (strict && stagedRoot.files) {
    for (const fileEntry of stagedRoot.files) {
      const resolvedEntry = join(RELEASE_DIR, "pruneguard", fileEntry);
      if (!existsSync(resolvedEntry)) {
        errs.push(`Root package "files" entry "${fileEntry}" does not exist in staged output`);
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

  // Verify all optionalDependency versions match the root version
  for (const [dep, v] of Object.entries(stagedRoot.optionalDependencies || {})) {
    if (v !== ver) {
      errs.push(`Root package optionalDependency "${dep}" has version "${v}" but expected "${ver}"`);
    }
  }

  // Check for banned patterns in root package.json
  errs.push(...checkForBannedPatterns(stagedRoot, "pruneguard"));

  // --- Check for banned artifacts across ALL staged packages ---
  const allEntryNames = walkEntryNames(RELEASE_DIR);
  for (const banned of BANNED_ENTRIES) {
    if (allEntryNames.includes(banned)) {
      errs.push(`Banned artifact "${banned}" found in staged output`);
    }
  }

  // --- Verify staged platform packages contain ONLY package.json + bin/ ---
  for (const dir of stagedPlatformDirs) {
    const dirPath = join(RELEASE_DIR, dir);
    const entries = readdirSync(dirPath);
    const unexpected = entries.filter((e) => e !== "package.json" && e !== "bin");
    for (const entry of unexpected) {
      errs.push(`${dir}: unexpected entry "${entry}" in staged platform package (only package.json and bin/ allowed)`);
    }
  }

  return errs;
}
