import { defineConfig } from 'vite';
import { iconsPlugin } from './src/vite-plugin-icons';

export default defineConfig({
  plugins: [iconsPlugin()],
});
