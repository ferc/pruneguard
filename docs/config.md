# Configuration

Most repos can run Pruneguard without a config file. The tool auto-detects
frameworks, discovers workspaces, and applies sensible defaults. Only create a
config file when you need custom rules, ownership settings, or framework
overrides.

## Config file resolution

No config file is required. When one is present, pruneguard reads JSON config
in this order:

1. Explicit `--config <path>` flag
2. `pruneguard.json` in the project root
3. `.pruneguardrc.json` in the project root

Run `pruneguard init` to generate a minimal `pruneguard.json` containing only
the `$schema` field. You can add sections as needed.

Run `pruneguard print-config` to see the fully resolved configuration.

## Schema

The configuration JSON schema is bundled at
`node_modules/pruneguard/configuration_schema.json`. Editors with JSON Schema
support will provide autocomplete and validation automatically when you add
the `$schema` field:

```json
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json"
}
```

To access the schema path programmatically:

```js
import { schemaPath } from "pruneguard";
console.log(schemaPath());
```

## Top-level sections

| Section | Purpose |
|---|---|
| `ignorePatterns` | Glob patterns for files to exclude from analysis |
| `workspaces` | Workspace roots and package manager |
| `resolver` | TypeScript and module resolution settings |
| `entrypoints` | Entrypoint discovery and overrides |
| `analysis` | Severity levels for each analyzer |
| `rules` | Forbidden and required import rules |
| `ownership` | CODEOWNERS integration and team config |
| `frameworks` | Framework auto-detection overrides |

## Example

```json
{
  "$schema": "./node_modules/pruneguard/configuration_schema.json",

  "ignorePatterns": ["**/dist/**", "**/node_modules/**"],

  "workspaces": {
    "packageManager": "pnpm",
    "roots": ["apps/*", "packages/*"]
  },

  "entrypoints": {
    "auto": true,
    "include": ["src/index.ts"],
    "exclude": ["**/*.test.ts"]
  },

  "analysis": {
    "unusedExports": "error",
    "unusedFiles": "warn",
    "unusedDependencies": "error",
    "unusedPackages": "warn",
    "cycles": "warn",
    "boundaries": "error",
    "ownership": "warn",
    "impact": "warn"
  },

  "frameworks": {
    "next": "auto",
    "vitest": "auto",
    "storybook": "auto"
  },

  "rules": {
    "forbidden": [
      {
        "name": "no-cross-app-imports",
        "severity": "error",
        "comment": "Apps must not import from other apps",
        "from": { "workspace": ["apps/*"] },
        "to": { "workspace": ["apps/*"] }
      }
    ]
  },

  "ownership": {
    "importCodeowners": true,
    "unownedSeverity": "warn"
  }
}
```

## Rule filters

Rules support the following filter fields:

| Filter | Matches against |
|---|---|
| `path` | File path (glob) |
| `pathNot` | Exclude file path (glob) |
| `workspace` | Workspace name (glob) |
| `workspaceNot` | Exclude workspace name (glob) |
| `package` | Package name (glob) |
| `packageNot` | Exclude package name (glob) |
| `tag` | Tag name |
| `tagNot` | Exclude tag name |
| `dependencyKinds` | Import kinds to match |
| `profiles` | Profiles to match (production, development, all) |
| `reachableFrom` | Must be reachable from matching nodes |
| `reaches` | Must reach matching nodes |
| `entrypointKinds` | Entrypoint kinds to match |

Reachability filters operate over graph nodes in the active profile. Query
values are matched against file paths, package names, and workspace names.

## Tag sources

Tags are assigned from three sources:

1. `ownership.teams[*].tags` -- applied to paths and packages matching the
   team definition
2. `overrides[*].tags` -- applied to files and workspaces matching the
   override
3. Implicit `entrypoint-kind:<kind>` tags derived from entrypoint detection

## Profiles

The `--profile` flag controls which entrypoints are active:

| Profile | Active entrypoints |
|---|---|
| `production` | Package exports, bin entries, framework pages |
| `development` | Test files, stories, fixture files |
| `all` | All detected entrypoints (default) |

## Analysis severity levels

Each analyzer can be set to one of:

- `"error"` -- reported as error, contributes to non-zero exit code
- `"warn"` -- reported as warning, contributes to non-zero exit code
- `"info"` -- reported as informational, does not affect exit code
- `"off"` -- disabled
