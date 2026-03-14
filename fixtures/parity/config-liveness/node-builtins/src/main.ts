import fs from 'node:fs';
import path from 'node:path';
import { AsyncLocalStorage } from 'node:async_hooks';
import { Channel } from 'node:diagnostics_channel';

export function readConfig() {
  const content = fs.readFileSync(path.join('.', 'config.json'), 'utf-8');
  const storage = new AsyncLocalStorage();
  const channel = new Channel('app');
  return { content, storage, channel };
}
