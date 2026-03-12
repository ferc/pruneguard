# pruneguard

Repo truth engine for JS/TS monorepos.

Build one accurate repo graph, then answer many high-value repo questions cheaply:
unused exports, unused files, unused dependencies, cycles, boundary violations,
ownership visibility, blast-radius analysis, and CI/agent-safe refactor checks.

## Install

```sh
npm install pruneguard
```

## Usage

```sh
# Full scan
pruneguard scan

# Focus findings to a slice of the repo
pruneguard --focus "src/**" scan

# Changed-since review for CI/agents
pruneguard --changed-since origin/main scan

# Deterministic CI/parity run without baseline influence
pruneguard --no-baseline --no-cache scan

# Fail advisory dead-code scans in automation
pruneguard --require-full-scope scan

# Partial-scope scan (advisory for dead-code findings)
pruneguard scan src/components/Button.tsx src/lib/utils.ts

# With config
pruneguard --config pruneguard.json scan

# Blast radius
pruneguard impact src/utils/helpers.ts

# Explain a finding
pruneguard explain unused-export:packages/core:src/old.ts#deprecatedFn

# Generate config
pruneguard init

# Debug resolution
pruneguard debug resolve ./utils --from src/index.ts
```

Dead-code trust model:

- full-repo `scan` is the trustworthy mode for deletion decisions
- `--focus` filters reported findings after full analysis
- positional `scan <paths...>` narrows the analyzed file set and is reported as partial-scope/advisory in the output
- `--require-full-scope` turns advisory partial-scope dead-code scans into a hard failure
- `--no-baseline` disables baseline auto-discovery for deterministic CI, parity, and benchmarks
- use `impact` and `explain` before removing code on unresolved-specifier-heavy repos

## Configuration

Create `pruneguard.json`:

```json
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json",
  "workspaces": {
    "packageManager": "pnpm",
    "roots": ["apps/*", "packages/*"]
  },
  "analysis": {
    "unusedExports": "error",
    "unusedFiles": "warn",
    "unusedDependencies": "error",
    "cycles": "warn"
  }
}
```

## Development

Requires: Rust (stable), Node.js, pnpm, just

```sh
just ready    # fmt + check + test + lint
just build    # release binary
just run scan # run against current directory
just schemas  # regenerate shipped schemas
just schemas-check
just build-js
just check-napi
just benchmark ../../claude-attack
just benchmark-repos
just parity   # opt-in real-repo smoke
```

## Experimental JS Exports

The npm package currently exposes these additional helpers as experimental:

- `scanDot`
- `migrateKnip`
- `migrateDepcruise`

They are usable now, but they are not yet treated as fully stable semver surface.

## License

MIT
