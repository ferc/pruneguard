export function createUnusedFixture() {
  return async ({ page }: { page: any }, use: any) => {
    await use(page);
  };
}
