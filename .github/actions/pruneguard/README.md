# pruneguard GitHub Action

A composite GitHub Action that installs and runs pruneguard in your CI
pipeline. Supports `scan`, `review`, `safe-delete`, and `fix-plan` commands
with JSON, text, or SARIF output.

## Quick Start: Branch Review

Block merges when pruneguard finds high-confidence issues on a branch:

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

      - uses: ./.github/actions/pruneguard
        id: review
        with:
          command: review
          args: "--changed-since origin/${{ github.base_ref }}"
          format: json
        continue-on-error: true

      - name: Check for blocking findings
        run: |
          if [ "${{ steps.review.outputs.exit-code }}" != "0" ]; then
            echo "::error::pruneguard review found blocking findings"
            cat "${{ steps.review.outputs.report-path }}"
            exit 1
          fi
```

## Full Scan in CI

Run a complete scan and upload the report as an artifact:

```yaml
name: pruneguard scan
on: push

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      - uses: ./.github/actions/pruneguard
        id: scan
        with:
          command: scan
          args: "--no-cache --no-baseline"
          format: json
        continue-on-error: true

      - uses: actions/upload-artifact@v4
        with:
          name: pruneguard-report
          path: ${{ steps.scan.outputs.report-path }}
```

## Safe-Delete Check

Verify that files are safe to remove before merging a deletion PR:

```yaml
name: safe-delete check
on: pull_request

jobs:
  safe-delete:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5

      - uses: ./.github/actions/pruneguard
        id: check
        with:
          command: safe-delete
          args: "src/legacy/old-module.ts src/deprecated/widget.ts"
          format: json
        continue-on-error: true

      - name: Verify deletions are safe
        run: |
          BLOCKED=$(echo '${{ steps.check.outputs.output }}' | jq '.blocked | length')
          if [ "$BLOCKED" != "0" ]; then
            echo "::error::Some targets are blocked from deletion"
            echo '${{ steps.check.outputs.output }}' | jq '.blocked'
            exit 1
          fi
```

## SARIF for GitHub Code Scanning

Upload pruneguard findings to the GitHub Security tab:

```yaml
name: pruneguard SARIF
on: push

jobs:
  scan:
    runs-on: ubuntu-latest
    permissions:
      security-events: write
    steps:
      - uses: actions/checkout@v5

      - uses: ./.github/actions/pruneguard
        id: scan
        with:
          command: scan
          args: "--no-cache --no-baseline"
          format: sarif
        continue-on-error: true

      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: ${{ steps.scan.outputs.report-path }}
          category: pruneguard
```

## Inputs

| Input               | Required | Default  | Description                                                    |
|---------------------|----------|----------|----------------------------------------------------------------|
| `command`           | Yes      |          | Command to run: `scan`, `review`, `safe-delete`, or `fix-plan` |
| `args`              | No       | `""`     | Additional CLI arguments                                       |
| `version`           | No       | `latest` | pruneguard version to install                                  |
| `format`            | No       | `json`   | Output format: `json`, `text`, or `sarif`                      |
| `working-directory` | No       | `.`      | Working directory for the pruneguard command                    |

## Outputs

| Output        | Description                                                          |
|---------------|----------------------------------------------------------------------|
| `exit-code`   | Exit code from pruneguard (0 = clean, 1 = findings/blockers)        |
| `output`      | Stdout from the command (truncated to 128KB for GitHub output limit) |
| `report-path` | Path to the report file (set for `json` and `sarif` formats)        |

## Tips

- Use `fetch-depth: 0` in `actions/checkout` when running `review` with
  `--changed-since` so that the git history is available for diff computation.
- Pin a specific version (`version: "0.2.1"`) for reproducible CI runs.
- Combine with `actions/upload-artifact` to preserve reports across jobs.
- Use `continue-on-error: true` on the action step and check
  `steps.<id>.outputs.exit-code` in a subsequent step for custom failure
  handling.
