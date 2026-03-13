import { test, expect } from '@playwright/test';

test('login page renders', async ({ page }) => {
  await page.goto('/login');
  await expect(page.locator('h1')).toHaveText('Login');
});

test('login with valid credentials', async ({ page }) => {
  await page.goto('/login');
  await page.fill('[name=email]', 'admin@example.com');
  await page.fill('[name=password]', 'secret');
  await page.click('button[type=submit]');
  await expect(page).toHaveURL('/dashboard');
});
