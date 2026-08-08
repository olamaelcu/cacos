import { test, expect } from '@playwright/test';

test.skip(!process.env.RUN_E2E, 'E2E is gated on RUN_E2E=1 and a running cacos PDS.');

test('consent flow redirects through to client callback', async ({ page }) => {
  // Start the consent flow by hitting the PDS authorize endpoint; the PDS
  // should 302 to https://localhost:5194/?rqid=...&state=...
  await page.goto(`${process.env.PDS_URL}/oauth/authorize?client_id=local&request_uri=urn:ietf:params:oauth:request_uri:manual`);
  // From here on depends on a seeded cacos PDS; the test is intentionally
  // short until Plan 06 has a seeded-DB helper in cacos/tests.
  expect(page.url()).toContain('localhost:5194');
});