// This regular .ts file has no importers and should be flagged as unused.
// It exists to verify that the ambient exclusion rule only applies to .d.ts files.
export function orphan(): string {
  return "no one imports this";
}
