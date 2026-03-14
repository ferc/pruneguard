import { seedDb } from './test-db';

export default async function globalSetup() {
  await seedDb();
}
