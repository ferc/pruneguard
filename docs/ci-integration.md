# CI Integration

pruneguard is designed for CI pipelines. It exits with deterministic codes,
produces structured output, and ships as a self-contained binary that
requires no Rust toolchain or Node.js runtime for the analysis itself.

## Exit codes

| Code | Meaning |
|------|---------|
| 0    | No findings (or no blocking findings for `review`) |
| 1    | Findings exist (or blocking findings for `review`) |
| 2    | Partial-scope scan when `--require-full-scope` is set |

## GitHub Action

The project includes a reusable composite action at
`.github/actions/pruneguard`. Use it in your workflows with:

```yaml
- uses: your-org/pruneguard/.github/actions/pruneguard@main
  with:
    command: review
    args: "--changed-since origin/${{ github.base_ref }}"
```

### Inputs

| Input               | Required | Default  | Description |
|---------------------|----------|----------|-------------|
| `command`           | Yes      |          | `scan`, `review`, `safe-delete`, or `fix-plan` |
| `args`              | No       | `""`     | Additional CLI arguments |
| `version`           | No       | `latest` | pruneguard version to install |
| `format`            | No       | `json`   | Output format: `json`, `text`, or `sarif` |
| `working-directory` | No       | `.`      | Working directory for the command |

### Outputs

| Output        | Description |
|---------------|-------------|
| `exit-code`   | 0 = clean, 1 = findings/blockers |
| `output`      | Stdout from the command (truncated to 128KB) |
| `report-path` | Path to the report file (json and sarif formats) |

---

## Branch review gate

The most common CI use case. Run `review` on every pull request and block
merges when high-confidence issues are found.

```yaml
name: pruneguard review
on: pull_request

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Branch review
        run: npx pruneguard --changed-since origin/${{ github.base_ref }} --format json review
```

Exit code 0 means no blocking findings. Exit code 1 means blocking findings
exist. The JSON output contains `blockingFindings` and `advisoryFindings`
arrays.

### Using the GitHub Action for branch review

```yaml
name: pruneguard review
on: pull_request

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: your-org/pruneguard/.github/actions/pruneguard@main
        id: review
        with:
          command: review
          args: "--changed-since origin/${{ github.base_ref }}"
        continue-on-error: true

      - name: Check results
        if: steps.review.outputs.exit-code != '0'
        run: |
          echo "::error::Blocking findings detected"
          cat "${{ steps.review.outputs.report-path }}"
          exit 1
```

---

## Safe-delete check

Verify that files deleted in a PR are safe to remove. Useful for automated
cleanup PRs or when removing legacy code.

```yaml
name: safe-delete check
on:
  pull_request:
    paths:
      - "**/*.ts"
      - "**/*.tsx"
      - "**/*.js"
      - "**/*.jsx"

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Check deletion safety
        run: |
          DELETED=$(git diff --name-only --diff-filter=D origin/${{ github.base_ref }}...HEAD \
            | grep -E '\.(ts|tsx|js|jsx|mts|mjs)$' || true)

          if [ -z "$DELETED" ]; then
            echo "No source files deleted in this PR."
            exit 0
          fi

          echo "Checking deletion safety for:"
          echo "$DELETED"
          npx pruneguard --format json safe-delete $DELETED
```

### Using the GitHub Action for safe-delete

```yaml
name: safe-delete check (action)
on:
  pull_request:
    paths:
      - "**/*.ts"
      - "**/*.tsx"
      - "**/*.js"
      - "**/*.jsx"

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - name: Find deleted files
        id: deleted
        run: |
          DELETED=$(git diff --name-only --diff-filter=D origin/${{ github.base_ref }}...HEAD \
            | grep -E '\.(ts|tsx|js|jsx|mts|mjs)$' || true)
          echo "files=$DELETED" >> "$GITHUB_OUTPUT"
          echo "has_deleted=$( [ -n "$DELETED" ] && echo true || echo false )" >> "$GITHUB_OUTPUT"

      - uses: your-org/pruneguard/.github/actions/pruneguard@main
        if: steps.deleted.outputs.has_deleted == 'true'
        id: safe-delete
        with:
          command: safe-delete
          args: "${{ steps.deleted.outputs.files }}"
        continue-on-error: true

      - name: Check results
        if: steps.deleted.outputs.has_deleted == 'true' && steps.safe-delete.outputs.exit-code != '0'
        run: |
          echo "::error::Some deleted files are not safe to remove"
          cat "${{ steps.safe-delete.outputs.report-path }}"
          exit 1
```

---

## Fix-plan in CI

Generate a remediation plan for findings and post it as a PR comment. Useful
for giving contributors actionable guidance.

```yaml
name: fix-plan
on: pull_request

jobs:
  plan:
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Generate fix plan
        id: plan
        run: |
          set +e
          OUTPUT=$(npx pruneguard --format json --changed-since origin/${{ github.base_ref }} fix-plan 2>&1)
          EXIT_CODE=$?

          if [ $EXIT_CODE -eq 0 ]; then
            ACTIONS=$(echo "$OUTPUT" | jq '.actions | length')
            if [ "$ACTIONS" != "0" ]; then
              {
                echo "body<<EOF"
                echo "## pruneguard fix plan"
                echo ""
                echo "$OUTPUT" | jq -r '.actions[] | "- **\(.kind)** \(.targets | join(", ")) (\(.risk) risk)\n  \(.why)"'
                echo ""
                echo "EOF"
              } >> "$GITHUB_OUTPUT"
            fi
          fi

      - name: Post PR comment
        if: steps.plan.outputs.body
        uses: marocchino/sticky-pull-request-comment@v2
        with:
          header: pruneguard-fix-plan
          message: ${{ steps.plan.outputs.body }}
```

### Using the GitHub Action for fix-plan

```yaml
name: fix-plan (action)
on: pull_request

jobs:
  plan:
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: your-org/pruneguard/.github/actions/pruneguard@main
        id: plan
        with:
          command: fix-plan
          args: "--changed-since origin/${{ github.base_ref }}"
        continue-on-error: true

      - name: Format and post comment
        if: steps.plan.outputs.exit-code == '0'
        run: |
          ACTIONS=$(jq '.actions | length' < "${{ steps.plan.outputs.report-path }}")
          if [ "$ACTIONS" != "0" ]; then
            {
              echo "body<<EOF"
              echo "## pruneguard fix plan"
              echo ""
              jq -r '.actions[] | "- **\(.kind)** \(.targets | join(", ")) (\(.risk) risk)\n  \(.why)"' < "${{ steps.plan.outputs.report-path }}"
              echo ""
              echo "EOF"
            } >> "$GITHUB_OUTPUT"
          fi

      - name: Post PR comment
        if: env.body
        uses: marocchino/sticky-pull-request-comment@v2
        with:
          header: pruneguard-fix-plan
          message: ${{ env.body }}
```

---

## Baseline-gated CI

Adopt pruneguard incrementally. Save a baseline on `main` and only fail on
new findings introduced by a PR.

```yaml
name: pruneguard baseline
on:
  push:
    branches: [main]
  pull_request:

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      # On main: save baseline
      - name: Save baseline
        if: github.ref == 'refs/heads/main'
        run: npx pruneguard --no-cache --no-baseline --format json scan > baseline.json

      - uses: actions/upload-artifact@v6
        if: github.ref == 'refs/heads/main'
        with:
          name: pruneguard-baseline
          path: baseline.json

      # On PRs: compare against baseline
      - uses: actions/download-artifact@v7
        if: github.event_name == 'pull_request'
        with:
          name: pruneguard-baseline
        continue-on-error: true

      - name: Check for new findings
        if: github.event_name == 'pull_request'
        run: |
          npx pruneguard --no-cache --no-baseline --format json scan > current.json
          node -e "
            const fs = require('fs');
            if (!fs.existsSync('baseline.json')) {
              console.log('No baseline found, skipping comparison');
              process.exit(0);
            }
            const baseline = JSON.parse(fs.readFileSync('baseline.json', 'utf-8'));
            const current = JSON.parse(fs.readFileSync('current.json', 'utf-8'));
            const baseIds = new Set(baseline.findings.map(f => f.id));
            const newFindings = current.findings.filter(f => !baseIds.has(f.id));
            if (newFindings.length > 0) {
              console.error(newFindings.length + ' new finding(s):');
              newFindings.forEach(f => console.error('  ' + f.id + ': ' + f.message));
              process.exit(1);
            }
            console.log('No new findings relative to baseline.');
          "
```

---

## SARIF for GitHub Code Scanning

Upload pruneguard findings to the GitHub Security tab using SARIF output:

```yaml
name: pruneguard SARIF
on:
  push:
    branches: [main]

jobs:
  scan:
    runs-on: ubuntu-latest
    permissions:
      security-events: write
    steps:
      - uses: actions/checkout@v5

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Scan
        run: npx pruneguard --no-cache --no-baseline --format sarif scan > results.sarif
        continue-on-error: true

      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: results.sarif
          category: pruneguard
```

---

## Monorepo: per-workspace scans

Run focused scans on specific workspace areas:

```yaml
name: workspace scan
on: pull_request

jobs:
  scan:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        workspace: [packages/core, packages/ui, apps/web]
    steps:
      - uses: actions/checkout@v5

      - uses: actions/setup-node@v6
        with:
          node-version: 24

      - run: npm install pruneguard

      - name: Scan workspace
        run: npx pruneguard --focus "${{ matrix.workspace }}/**" --format json scan
```

---

## CI environment detection

pruneguard automatically detects CI environments (`CI=true`) and defaults to
one-shot mode (no daemon). This means:

- Every invocation does a fresh graph build
- Results are fully deterministic
- No background process is left running

You can explicitly control this with `--daemon off` or `--daemon auto`.

## Tips

- Always use `fetch-depth: 0` in `actions/checkout` when using
  `--changed-since` so that full git history is available.
- Use `--no-cache --no-baseline` for the most deterministic results.
- Pin a specific version (`npm install pruneguard@0.3.0`) for reproducible
  CI runs.
- Use `--format sarif` to feed results into GitHub Code Scanning.
- Use `continue-on-error: true` on the pruneguard step and check the exit
  code in a subsequent step for custom failure handling.
