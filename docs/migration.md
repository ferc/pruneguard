# Migration

pruneguard includes built-in config converters for knip and dependency-cruiser.
Both produce a `pruneguard.json` with migration notes so you can switch tools
without starting from scratch.

## From knip

```sh
pruneguard migrate knip [file]
```

Config discovery:

1. Uses the explicit `file` argument if provided
2. Otherwise looks for `knip.json`
3. Otherwise looks for `package.json#knip`

Mapping:

| knip | pruneguard |
|---|---|
| `entry` | `entrypoints.include` |
| `project` | `ignorePatterns` (inverted) |
| `workspaces` | `workspaces.roots` |
| `ignore` | `ignorePatterns` |
| `ignoreDependencies` | `analysis.ignoreDependencies` |

Known limitations are emitted as warnings in the migration output.

### JS API

```js
import { migrateKnip } from "pruneguard";

const result = await migrateKnip({ file: "knip.json" });
console.log(JSON.stringify(result.config, null, 2));
for (const w of result.warnings) {
  console.warn(`warning: ${w}`);
}
```

## From dependency-cruiser

```sh
pruneguard migrate depcruise [file]
```

Config discovery:

1. Uses the explicit `file` argument if provided
2. Otherwise searches common `.dependency-cruiser.*` filenames
3. Supports `--node` for evaluating dynamic config files (JS/TS configs)

Mapping:

| dependency-cruiser | pruneguard |
|---|---|
| `forbidden` | `rules.forbidden` |
| `required` | `rules.required` |
| `options.tsConfig` | `resolver.tsconfig` |
| `options.exclude` | `ignorePatterns` |

Known limitations are emitted as warnings in the migration output.

### JS API

```js
import { migrateDepcruise } from "pruneguard";

const result = await migrateDepcruise({ node: true });
console.log(JSON.stringify(result.config, null, 2));
for (const w of result.warnings) {
  console.warn(`warning: ${w}`);
}
```

## Post-migration workflow

1. Run the migration command to generate `pruneguard.json`.
2. Review the generated config and any warnings.
3. Run `pruneguard scan` to see the initial findings.
4. Adjust severity levels and ignore patterns as needed.
5. Save a baseline: `pruneguard --no-cache --no-baseline --format json scan > baseline.json`
6. Commit `pruneguard.json` and `baseline.json`.
7. Remove the old tool's config file when ready.
