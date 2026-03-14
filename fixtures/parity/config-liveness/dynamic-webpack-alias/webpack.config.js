const path = require('path');

// This pattern uses a variable intermediate, which the static config
// reader cannot follow (it can only evaluate path.resolve with literal args).
const ROOT = path.resolve(__dirname, '..');
const SRC = path.join(ROOT, 'src');

module.exports = {
  entry: './src/main.ts',
  resolve: {
    alias: {
      '@ds': path.join(SRC, 'design-system'),
    },
  },
};
