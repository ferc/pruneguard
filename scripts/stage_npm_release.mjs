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
 */

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const NPM_SRC = join(ROOT, "npm");
const RELEASE_DIR = join(ROOT, ".release", "npm");

// ---------------------------------------------------------------------------
// Parse args
// ---------------------------------------------------------------------------

let versionOverride;
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--version" && args[i + 1]) {
    versionOverride = args[++i];
  }
}

// ---------------------------------------------------------------------------
// Read source version
// ---------------------------------------------------------------------------

const rootPkg = JSON.parse(readFileSync(join(NPM_SRC, "pruneguard", "package.json"), "utf8"));
const version = versionOverride ?? rootPkg.version;

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

// Remove node_modules from staged copy
const stagedNodeModules = join(rootDst, "node_modules");
if (existsSync(stagedNodeModules)) {
  rmSync(stagedNodeModules, { recursive: true });
}

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

  // Remove node_modules from staged copy
  const nm = join(dst, "node_modules");
  if (existsSync(nm)) rmSync(nm, { recursive: true });

  const pkgPath = join(dst, "package.json");
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));

  // Sync version
  pkg.version = version;

  // Inject shared metadata
  Object.assign(pkg, sharedMeta);

  // Set repository.directory for this specific package
  pkg.repository = {
    ...pkg.repository,
    directory: `npm/${dir}`,
  };

  writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
  console.log(`  ✓ ${pkg.name}@${version}`);
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

console.log(`\nStaged ${1 + platformDirs.length} packages into ${RELEASE_DIR}`);

// Verify no workspace: references remain
const verifyPkg = JSON.parse(readFileSync(join(rootDst, "package.json"), "utf8"));
for (const field of ["dependencies", "optionalDependencies", "peerDependencies", "devDependencies"]) {
  for (const [dep, ver] of Object.entries(verifyPkg[field] || {})) {
    if (ver.startsWith("workspace:")) {
      console.error(`ERROR: staged root package still has workspace: reference: ${dep}@${ver}`);
      process.exit(1);
    }
  }
}
console.log("Verified: no workspace: references in staged root package.");
