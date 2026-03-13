import { configure } from './utils';

configure({ debug: false });

// Polyfill for TextEncoder in jsdom
if (typeof globalThis.TextEncoder === 'undefined') {
  globalThis.TextEncoder = require('util').TextEncoder;
}
