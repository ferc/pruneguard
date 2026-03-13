import { defineConfig } from 'vite';

export default defineConfig({
  resolve: {
    alias: {
      '@utils': './src/utils',
    },
  },
});
