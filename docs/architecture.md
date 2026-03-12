# Architecture

`pruneguard` builds one repository graph and reuses it for every analysis:

1. discovery
2. file collection and classification
3. extraction
4. resolution
5. module graph and symbol graph build
6. entrypoint detection
7. analyzers
8. report rendering

Core design rules:

- the hot path stays in Rust
- the graph is built once per run
- analyzers reuse graph indices instead of rewalking the repo
- findings and output ordering are deterministic
- Node exists only as packaging and wrapper glue
