export function getDb() {
  return {
    query: (sql: string) => Promise.resolve([]),
    close: () => Promise.resolve(),
  };
}
