// Main CLI entry — reads template files at runtime via fs, not import.
import { readFileSync } from 'node:fs';

export function scaffold(templateName: string): string {
  const content = readFileSync(`./template/extras/${templateName}`, 'utf-8');
  return content;
}
