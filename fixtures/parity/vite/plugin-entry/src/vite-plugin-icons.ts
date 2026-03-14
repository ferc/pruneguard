import { loadIcon } from './icon-loader';
import type { Plugin } from 'vite';

export function iconsPlugin(): Plugin {
  return {
    name: 'vite-plugin-icons',
    resolveId(id) {
      if (id.startsWith('virtual:icon/')) return id;
    },
    load(id) {
      if (id.startsWith('virtual:icon/')) {
        return loadIcon(id.replace('virtual:icon/', ''));
      }
    },
  };
}
