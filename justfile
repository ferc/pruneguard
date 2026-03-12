# oxgraph development commands

# Run all checks (format, check, test, lint)
ready: fmt check test lint
alias r := ready

# Format all code
fmt:
    cargo fmt --all

# Check the workspace
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run clippy
lint:
    cargo clippy --workspace -- --deny warnings

# Build release binary
build:
    cargo build --release -p oxgraph

# Run oxgraph on a target directory
run *args:
    cargo run -p oxgraph -- {{args}}

# Generate JSON schemas
schemas:
    cargo run -p oxgraph --bin generate_schemas

# Watch for changes and re-check
watch cmd="check":
    cargo watch -x "{{cmd}}"

# Run microbenchmarks
benchmark-workspace:
    cargo bench --workspace

# Run one named corpus scan in release mode
benchmark CASE:
    cargo run --release -p oxgraph --bin oxgraph -- --format json scan {{CASE}}

# Run configured corpus scans in release mode
benchmark-repos:
    cargo test -p oxgraph parity_smoke -- --ignored --nocapture

# Run one fixture scan smoke
fixture CASE:
    cargo test -p oxgraph scan_smoke -- --nocapture {{CASE}}

# Run real-repo smoke on configured corpora
smoke-repos:
    cargo test -p oxgraph parity_smoke -- --ignored --nocapture

# Run package smoke locally
pack-smoke:
    mkdir -p /tmp/oxgraph-pack
    npm_config_cache=/tmp/oxgraph-npm-cache npm pack --prefix npm/oxgraph --pack-destination /tmp/oxgraph-pack

# Alias for parity harness
parity:
    cargo test -p oxgraph parity_smoke -- --ignored --nocapture

parity-repos:
    cargo test -p oxgraph parity_smoke -- --ignored --nocapture
