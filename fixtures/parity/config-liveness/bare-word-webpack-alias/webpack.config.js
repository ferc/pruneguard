const path = require('path');

// Bare-word aliases configured via dynamic path.resolve with variables.
// The static config reader cannot extract these since ROOT is a variable.
const ROOT = path.resolve(__dirname, '..');

module.exports = {
  entry: './src/main.ts',
  resolve: {
    alias: {
      base: path.join(ROOT, 'src/styles/base'),
      theme: path.join(ROOT, 'src/styles/theme'),
      layout: path.join(ROOT, 'src/styles/layout'),
      vendor: path.join(ROOT, 'vendor'),
    },
  },
};
