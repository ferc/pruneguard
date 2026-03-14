import { log } from '$lib/logger';
import type { Handle } from '@sveltejs/kit';

export const handle: Handle = async ({ event, resolve }) => {
  log(`${event.request.method} ${event.url.pathname}`);
  return resolve(event);
};
