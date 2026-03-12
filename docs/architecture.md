# Architecture

pruneguard builds one repository graph and reuses it for every analysis.

## Pipeline stages

1. **Discovery** -- find workspaces, packages, and source files
2. **File collection and classification** -- categorize files by role (source, test, story, config, etc.)
3. **Extraction** -- parse each file and extract imports, exports, and symbols
4. **Resolution** -- resolve import specifiers to concrete file paths
5. **Module graph and symbol graph build** -- construct the complete dependency graph
6. **Entrypoint detection** -- identify package exports, bin entries, framework conventions
7. **Analyzers** -- run all enabled analyzers over the graph in a single pass
8. **Report rendering** -- produce findings with evidence, stats, and trust metadata

## Runtime model

The `pruneguard` npm package ships a compiled Rust binary for each supported
platform. The JS wrapper spawns this binary and parses its JSON output.

```
npm install pruneguard
       |
       v
@pruneguard/cli-<platform>   <-- native binary, auto-selected
       |
       v
pruneguard (JS wrapper)      <-- spawns binary, parses JSON
       |
       +-- CLI: npx pruneguard scan
       +-- JS API: import { scan } from "pruneguard"
```

**Execution modes:**

| Mode | Default in | Behavior |
|---|---|---|
| Daemon | Local terminal | Graph stays warm in memory, sub-ms queries |
| One-shot | CI environments | Fresh graph per invocation, deterministic |

## Design rules

- The hot path stays in Rust (parsing, resolution, graph build, analysis)
- The graph is built once per run
- Analyzers reuse graph indices instead of rewalking the repo
- Findings and output ordering are deterministic
- Node.js exists only as packaging and wrapper glue
- The binary has zero Node.js runtime dependency
