import type { Plugin } from 'vite';

export function unusedPlugin(): Plugin {
  return { name: 'unused-plugin' };
}
