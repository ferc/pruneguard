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
    cargo run -p oxgraph -- print-config > /dev/null
    @echo "Schemas generated"

# Watch for changes and re-check
watch cmd="check":
    cargo watch -x "{{cmd}}"

# Run benchmarks
benchmark:
    cargo bench --workspace
