import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte({ hot: false })],
  resolve: {
    conditions: ['browser'],
  },
  test: {
    include: ['tests/unit/**/*.test.ts'],
    setupFiles: ['tests/setup.ts'],
    environmentMatchGlobs: [
      ['tests/unit/screens.test.ts', 'jsdom'],
      ['tests/unit/pds/**', 'node'],
      ['tests/unit/{env,hooks,passkey.api,repo-status}.test.ts', 'node'],
    ],
  },
});
