# pruneguard development commands

# Configure git hooks (run once after clone)
setup:
    git config core.hooksPath .githooks

# Run all checks (format, check, test, lint)
ready: fmt check test lint
alias r := ready

# Run the full CI pipeline locally (mirrors .github/workflows/ci.yml)
ci: fmt check lint test build-js schemas-check stage-release verify-staged

# Format all code
fmt:
    cargo fmt --all

# Check the workspace
check:
    cargo check --workspace

# Check the N-API feature build (internal only)
check-napi:
    cargo check -p pruneguard --features napi

# Run all tests
test:
    cargo test --workspace

# Run clippy
lint:
    cargo clippy --workspace -- --deny warnings

# Build release binary
build:
    cargo build --release -p pruneguard

# Run pruneguard on a target directory
run *args:
    cargo run -p pruneguard -- {{args}}

# Generate JSON schemas
schemas:
    cargo run -p pruneguard --bin generate_schemas

# Verify generated schemas are committed
schemas-check:
    cargo run -p pruneguard --bin generate_schemas
    git diff --exit-code -- npm/pruneguard/configuration_schema.json npm/pruneguard/report_schema.json

# Build the JS wrapper only
build-js:
    pnpm --dir apps/pruneguard build-js

# Watch for changes and re-check
watch cmd="check":
    cargo watch -x "{{cmd}}"

# Run microbenchmarks
benchmark-workspace:
    cargo bench --workspace

# Run one named corpus scan in release mode
benchmark CASE:
    cargo run --release -p pruneguard --bin pruneguard -- --format json --no-cache --no-baseline scan {{CASE}}

# Run configured corpus scans in release mode
benchmark-repos:
    cargo test -p pruneguard parity_smoke -- --ignored --nocapture

# Run one fixture scan smoke
fixture CASE:
    cargo test -p pruneguard scan_smoke -- --nocapture {{CASE}}

# Run real-repo smoke on configured corpora
smoke-repos:
    cargo test -p pruneguard parity_smoke -- --ignored --nocapture

# Stage npm packages for release (rewrites workspace:* to concrete versions)
stage-release:
    node scripts/stage_npm_release.mjs

# Verify staged npm packages without re-staging
verify-staged:
    node scripts/stage_npm_release.mjs --verify

# Build artifacts needed for pack-smoke
_pack-smoke-build: build build-js schemas stage-release

# Run package smoke locally (builds binary + JS, stages into .release, packs and tests)
pack-smoke: _pack-smoke-build
    #!/usr/bin/env bash
    set -euo pipefail
    # Detect current platform package dir
    ARCH=$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x64/')
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    PLATFORM_DIR="cli-${OS}-${ARCH}"
    if [ "$OS" = "linux" ]; then
      if ldd --version 2>&1 | grep -q musl; then
        PLATFORM_DIR="${PLATFORM_DIR}-musl"
      else
        PLATFORM_DIR="${PLATFORM_DIR}-gnu"
      fi
    fi
    # Stage binary into the release platform package
    mkdir -p ".release/npm/${PLATFORM_DIR}/bin"
    cp target/release/pruneguard ".release/npm/${PLATFORM_DIR}/bin/"
    # Pack staged packages
    PACK_DIR="/tmp/pruneguard-pack-$$"
    rm -rf "$PACK_DIR"
    mkdir -p "$PACK_DIR"
    npm pack --pack-destination "$PACK_DIR" ".release/npm/${PLATFORM_DIR}"
    npm pack --pack-destination "$PACK_DIR" ".release/npm/pruneguard"
    # Install into temp project and verify
    SMOKE_DIR="/tmp/pruneguard-smoke-$$"
    rm -rf "$SMOKE_DIR"
    mkdir -p "$SMOKE_DIR"
    cd "$SMOKE_DIR"
    npm init -y > /dev/null 2>&1
    PLATFORM_TGZ=$(ls "$PACK_DIR"/pruneguard-cli-*.tgz 2>/dev/null | head -n1)
    ROOT_TGZ=$(ls "$PACK_DIR"/pruneguard-0*.tgz 2>/dev/null | head -n1)
    if [ -z "$PLATFORM_TGZ" ] || [ -z "$ROOT_TGZ" ]; then
      echo "ERROR: packed tarballs not found in $PACK_DIR"
      ls "$PACK_DIR"
      exit 1
    fi
    npm install "$PLATFORM_TGZ" "$ROOT_TGZ"
    echo "--- Verifying binaryPath() ---"
    node -e "import('pruneguard').then(m => console.log('binaryPath:', m.binaryPath()))"
    echo "--- Verifying --help ---"
    npx pruneguard --help
    echo "--- Verifying scan on fixture project ---"
    mkdir -p "$SMOKE_DIR/fixture/src"
    cat > "$SMOKE_DIR/fixture/package.json" <<'FIXTURE_PKG'
    { "name": "fixture", "version": "0.0.0", "private": true }
    FIXTURE_PKG
    cat > "$SMOKE_DIR/fixture/src/index.ts" <<'FIXTURE_INDEX'
    import { helper } from "./used";
    console.log(helper());
    FIXTURE_INDEX
    cat > "$SMOKE_DIR/fixture/src/used.ts" <<'FIXTURE_USED'
    export function helper() { return 42; }
    FIXTURE_USED
    cat > "$SMOKE_DIR/fixture/src/unused.ts" <<'FIXTURE_UNUSED'
    export function neverCalled() { return "dead code"; }
    FIXTURE_UNUSED
    echo "  -> CLI scan"
    CLI_OUT=$(cd "$SMOKE_DIR/fixture" && npx pruneguard --format json --no-cache --no-baseline scan) || [ $? -eq 1 ]
    echo "$CLI_OUT" | node -e "
      let buf=''; process.stdin.on('data',c=>buf+=c); process.stdin.on('end',()=>{
        const j=JSON.parse(buf);
        if(!j.findings||j.findings.length===0){console.error('Expected findings in CLI output');process.exit(1);}
        console.log('CLI scan found',j.findings.length,'findings');
      });"
    echo "  -> JS scan() API"
    node -e "import('pruneguard').then(m => m.scan({ cwd: '$SMOKE_DIR/fixture' }).then(r => { if (r.findings.length === 0) { console.error('Expected findings'); process.exit(1); } console.log('scan() found', r.findings.length, 'findings'); }))"
    echo "--- Pack smoke passed ---"
    rm -rf "$PACK_DIR" "$SMOKE_DIR"

# Alias for parity harness
parity:
    cargo test -p pruneguard parity_smoke -- --ignored --nocapture

parity-repos:
    cargo test -p pruneguard parity_smoke -- --ignored --nocapture
