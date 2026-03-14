export function createAuthFixture() {
  return async ({ page }: { page: any }, use: any) => {
    await page.goto('/login');
    await use(page);
  };
}
