// Webpack inline loader syntax uses ! separators.
// These must not leak as unlisted dependencies.
const raw = require('!!raw-loader!./file.txt');
const custom = require('!my-loader!.');

export { raw, custom };
