import { getDb } from '../utils/db';

export default defineEventHandler(() => {
  return getDb().getUsers();
});
