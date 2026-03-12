#!/usr/bin/env bash
# Publish an npm package only if its version is not already on the registry.
#
# This script expects to publish from .release/npm/ where workspace:*
# references have already been rewritten by stage_npm_release.mjs.
# It will refuse to publish if any workspace: references remain.
#
# Usage: bash .github/scripts/publish-if-needed.sh <pkg_dir> [publish_flags...]
# Example: bash .github/scripts/publish-if-needed.sh .release/npm/pruneguard --provenance --access public --no-git-checks

set -euo pipefail

pkg_dir="$1"
shift

read -r name version < <(node -e "const p=require('./${pkg_dir}/package.json'); console.log(p.name+' '+p.version)")

npm_stderr=$(mktemp)
if published=$(npm view "${name}@${version}" version 2>"$npm_stderr"); then
  if [ "$published" = "$version" ]; then
    rm -f "$npm_stderr"
    echo "SKIP: ${name}@${version} already published."
    exit 0
  fi
else
  # npm view failed — log the error but continue to publish (which will
  # produce a clear error if the registry is actually unreachable).
  echo "WARNING: npm view ${name}@${version} failed:"
  cat "$npm_stderr" >&2
fi
rm -f "$npm_stderr"

# Guard: refuse to publish if workspace: references remain.
# The staging script should have already rewritten these.
node -e "
  const fs = require('fs');
  const p = JSON.parse(fs.readFileSync('./${pkg_dir}/package.json', 'utf8'));
  let found = false;
  for (const field of ['dependencies','optionalDependencies','peerDependencies','devDependencies']) {
    for (const [dep, ver] of Object.entries(p[field] || {})) {
      if (typeof ver === 'string' && ver.startsWith('workspace:')) {
        console.error('ERROR: ' + p.name + ' has workspace: reference: ' + dep + '@' + ver);
        found = true;
      }
    }
  }
  if (found) {
    console.error('Refusing to publish — run stage_npm_release.mjs first.');
    process.exit(1);
  }
"

npm publish "${pkg_dir}/" "$@"
