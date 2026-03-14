import { getDb } from '../../src/db';

export default async function handler(req: any, res: any) {
  const db = getDb();
  res.json({ users: db.getUsers() });
}
