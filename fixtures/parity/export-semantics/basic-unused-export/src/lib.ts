export function liveExport(): string {
  return 'consumed by index.ts';
}

export function deadExport(): string {
  return 'never imported by anyone';
}
