// User code that does NOT import svelte or @sveltejs/kit directly.
// The framework consumes these at build time via svelte.config.js.
export function greet(name: string): string {
  return `Hello, ${name}!`;
}
