// This file is not directly imported, but the project has unresolved
// dynamic loader imports. Under unresolved pressure, the confidence
// of this finding should be less than "high".
export function maybeUsed(): void {
  console.log("might be loaded dynamically");
}
