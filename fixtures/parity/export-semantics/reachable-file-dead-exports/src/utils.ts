// Utility file with mixed usage — some exports consumed, others dead.

export const USED_CONSTANT = 42;
export const UNUSED_CONSTANT = 99;

export function usedHelper(n: number): string {
  return `value: ${n}`;
}

export function unusedHelper(): void {
  console.log('never called by anyone');
}
