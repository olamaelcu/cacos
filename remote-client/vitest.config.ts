import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  plugins: [svelte({ hot: false })],
  resolve: {
    conditions: ['browser'],
    alias: {
      $lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
      'server-only': fileURLToPath(new URL('./tests/stubs/server-only.ts', import.meta.url)),
    },
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
