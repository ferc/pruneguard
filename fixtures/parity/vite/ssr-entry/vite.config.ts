import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    ssr: './src/entry-server.ts',
  },
});
