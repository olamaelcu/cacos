import { test, expect } from '@playwright/test';

test.skip(!process.env.RUN_E2E, 'Mock-mode e2e requires RUN_E2E=1; no real PDS needed.');

test.describe('mock mode (no PDS required)', () => {
  test('renders each PDS screen via ?mock=', async ({ page }) => {
    for (const screen of ['sign-in', 'select', 'consent', 'create', 'error'] as const) {
      await page.goto(`/?mock=${screen}`);
      await expect(page.getByRole('heading')).toBeVisible();
    }
  });

  test('sign-in screen has identifier + password fields', async ({ page }) => {
    await page.goto('/?mock=sign-in');
    await expect(page.locator('wa-input[name=identifier]')).toHaveCount(1);
    await expect(page.locator('wa-input[name=password]')).toHaveCount(1);
    await expect(page.locator('input[name=state]')).toHaveValue('mock-state');
  });

  test('select screen lists two sessions', async ({ page }) => {
    await page.goto('/?mock=select');
    const radios = page.locator('wa-radio');
    await expect(radios).toHaveCount(2);
    await expect(radios.first()).toContainText('alice.mock');
  });

  test('consent screen shows scopes and Allow/Deny', async ({ page }) => {
    await page.goto('/?mock=consent');
    await expect(page.getByText('atproto')).toBeVisible();
    await expect(page.getByText('transition:email')).toBeVisible();
    await expect(page.locator('form[action="?/accept"]')).toHaveCount(1);
    await expect(page.locator('form[action="?/reject"]')).toHaveCount(1);
  });

  test('error screen surfaces description', async ({ page }) => {
    await page.goto('/?mock=error');
    await expect(page.getByText(/Demo error/)).toBeVisible();
  });

  test('account page renders with ?mock', async ({ page }) => {
    await page.goto('/account?mock');
    await expect(page.getByText('alice.mock')).toBeVisible();
    await expect(page.getByText('did:plc:mockaliceabcdef1234567890')).toBeVisible();
  });

  test('splash page shows dev-only mock triggers with links to every screen', async ({ page }) => {
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: /Mock flow/i });
    await expect(nav).toBeVisible();
    for (const screen of ['sign-in', 'select', 'consent', 'create', 'error']) {
      await expect(nav.locator(`a[href="/?mock=${screen}"], wa-button[href="/?mock=${screen}"]`)).toHaveCount(1);
    }
    await expect(nav.locator('a[href="/account?mock"], wa-button[href="/account?mock"]')).toHaveCount(1);
    await expect(nav.locator('a[href="/mock-client"], wa-button[href="/mock-client"]')).toHaveCount(1);
  });

  test('mock triggers disappear on /?mock=consent (not in splash branch)', async ({ page }) => {
    await page.goto('/?mock=consent');
    await expect(page.getByRole('navigation', { name: /Mock flow/i })).toHaveCount(0);
  });

  test('sign-in submit chains to consent', async ({ page }) => {
    await page.goto('/?mock=sign-in');
    await page.locator('wa-input[name=identifier]').fill('alice');
    await page.locator('wa-input[name=password]').fill('pw');
    await Promise.all([
      page.waitForURL(/\/\?mock=consent/),
      page.locator('form[action="?/signIn"]').evaluate((f: HTMLFormElement) => f.submit()),
    ]);
    await expect(page.getByText('atproto')).toBeVisible();
  });

  test('consent Allow chains to /mock-client success page', async ({ page }) => {
    await page.goto('/?mock=consent');
    await Promise.all([
      page.waitForURL(/\/mock-client\?code=/),
      page.locator('form[action="?/accept"]').evaluate((f: HTMLFormElement) => f.submit()),
    ]);
    await expect(page.getByText('Authorization granted')).toBeVisible();
    await expect(page.getByText('mock-code')).toBeVisible();
  });

  test('consent Deny chains to /mock-client error page', async ({ page }) => {
    await page.goto('/?mock=consent');
    await Promise.all([
      page.waitForURL(/\/mock-client\?error=/),
      page.locator('form[action="?/reject"]').evaluate((f: HTMLFormElement) => f.submit()),
    ]);
    await expect(page.getByText('Authorization denied')).toBeVisible();
    await expect(page.getByText('access_denied')).toBeVisible();
  });

  test('mock-client without params shows neutral placeholder', async ({ page }) => {
    await page.goto('/mock-client');
    await expect(page.getByText(/No callback params/)).toBeVisible();
    await expect(page.locator('a[href="/?mock=consent"], wa-button[href="/?mock=consent"]').first()).toBeVisible();
  });
});
