# oxgraph

Repo truth engine for JS/TS monorepos.

Build one accurate repo graph, then answer many high-value repo questions cheaply:
unused exports, unused files, unused dependencies, cycles, boundary violations,
ownership visibility, blast-radius analysis, and CI/agent-safe refactor checks.

## Install

```sh
npm install oxgraph
```

## Usage

```sh
# Full scan
oxgraph scan

# With config
oxgraph --config oxgraph.json scan

# Blast radius
oxgraph impact src/utils/helpers.ts

# Explain a finding
oxgraph explain unused-export:packages/core:src/old.ts#deprecatedFn

# Generate config
oxgraph init

# Debug resolution
oxgraph debug-resolve ./utils --from src/index.ts
```

## Configuration

Create `oxgraph.json`:

```json
{
  "$schema": "./node_modules/oxgraph/configuration_schema.json",
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
```

## License

MIT
