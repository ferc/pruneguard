# Migration

`pruneguard migrate knip [file]`:

- loads the explicit file if provided
- otherwise looks for `knip.json`
- otherwise looks for `package.json#knip`

`pruneguard migrate depcruise [file]`:

- loads the explicit file if provided
- otherwise searches the common `.dependency-cruiser.*` filenames
- supports `--node` only for evaluating dynamic config files

Current mapping highlights:

- Knip `entry` -> `entrypoints.include`
- Knip `workspaces` -> `workspaces.roots`
- dependency-cruiser `forbidden` -> `rules.forbidden`
- dependency-cruiser `required` -> `rules.required`
- dependency-cruiser `options.tsConfig` -> `resolver.tsconfig`

Known limitations are emitted as warnings in the migration output.
