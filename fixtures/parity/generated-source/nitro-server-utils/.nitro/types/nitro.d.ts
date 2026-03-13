export { getDb } from '../../server/utils/db';

declare module 'nitropack' {
  interface NitroUtils {
    getDb: typeof import('../../server/utils/db')['getDb'];
  }
}
