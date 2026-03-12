# Operational

Operational defaults:

- cache path: `.oxgraph/cache.redb`
- baseline search order:
  - config directory
  - project root
- focus filtering:
  - full graph still builds
  - only returned findings/proofs/impact sets are narrowed

Useful commands:

```sh
just ready
just parity
just smoke-repos
pnpm -r build
npm pack --prefix npm/oxgraph --pack-destination /tmp/oxgraph-pack
```
