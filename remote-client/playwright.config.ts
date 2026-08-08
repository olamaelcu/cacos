import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  timeout: 30_000,
  reporter: process.env.RUN_E2E ? 'list' : [['line']],
  use: {
    baseURL: 'https://localhost:5194',
    ignoreHTTPSErrors: true,
  },
});