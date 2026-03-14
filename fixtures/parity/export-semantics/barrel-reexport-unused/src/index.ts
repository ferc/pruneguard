// Barrel file that re-exports from submodules.
// usedFn is consumed by main.ts, unusedFn is not consumed by anyone.
export { usedFn } from './used';
export { unusedFn } from './unused';
