import { defineConfig } from '@playwright/test';

export default defineConfig({
  globalSetup: './src/global-setup.ts',
  globalTeardown: './src/global-teardown.ts',
  testDir: './tests',
});
