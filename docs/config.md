# Config

`oxgraph` reads JSON config in this order:

1. explicit `--config`
2. `oxgraph.json`
3. `.oxgraphrc.json`

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
- `dependencyKinds`
- `profiles`
- `entrypointKinds`

Known unsupported future rule fields:

- `tag`
- `tagNot`
- `reachableFrom`
- `reaches`

Unsupported rule fields fail explicitly instead of being ignored.
