// Platform-specific protocol imports should not be flagged as unlisted.
import { env } from 'cloudflare:workers';

export function handler() {
  return env;
}
