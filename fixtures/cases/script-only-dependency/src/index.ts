// This file does NOT import typescript.
// The "typescript" package is used only in package.json scripts ("tsc").
// pruneguard should recognize script-level usage and not flag it as unused.
export function main(): string {
  return "hello";
}
