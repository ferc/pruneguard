export function usedExternally() {
  return 'external';
}

export function usedInternally() {
  return 'internal';
}

export function neverReferenced() {
  return 'dead';
}

// Internal usage of the exported function
const result = usedInternally();
console.log(result);
