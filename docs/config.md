# Config

`pruneguard` reads JSON config in this order:

1. explicit `--config`
2. `pruneguard.json`
3. `.pruneguardrc.json`

Supported top-level areas in the current implementation:

- `ignorePatterns`
- `workspaces`
- `resolver`
- `entrypoints`
- `analysis`
- `rules`
- `ownership`
- `frameworks`

Current rule filter support:

- `path`
- `pathNot`
- `workspace`
- `workspaceNot`
- `package`
- `packageNot`
- `tag`
- `tagNot`
- `dependencyKinds`
- `profiles`
- `reachableFrom`
- `reaches`
- `entrypointKinds`

Current tag sources:

- `ownership.teams[*].tags` for matched paths and packages
- `overrides[*].tags` for matched `files` and `workspaces`
- implicit `entrypoint-kind:<kind>` tags derived from entrypoint detection

Reachability filters operate over graph nodes in the active profile. Query values are matched
against file paths, package names, and workspace names.
