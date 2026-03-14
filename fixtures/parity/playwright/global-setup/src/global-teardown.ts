import { cleanupDb } from './test-db';

export default async function globalTeardown() {
  await cleanupDb();
}
