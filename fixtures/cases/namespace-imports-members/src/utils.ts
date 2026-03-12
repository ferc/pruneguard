export function foo(): string {
  return "used via namespace access";
}

export function bar(): number {
  return 0; // never accessed — should be flagged as unused export
}
