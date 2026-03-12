// This file is outside the partial scan scope.
// It should trigger a partial-scope advisory when scanning only a subset.
export function other(): string {
  return "outside scan scope";
}
