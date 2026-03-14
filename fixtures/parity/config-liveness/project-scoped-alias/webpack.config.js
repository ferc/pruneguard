const path = require('path');

// Project-specific scoped aliases that are NOT real npm organizations.
// These are internal directory aliases configured via webpack, not packages.
const ROOT = path.resolve(__dirname, '..');

module.exports = {
  entry: './src/main.ts',
  resolve: {
    alias: {
      '@artifacts': path.join(ROOT, 'src/features/artifacts'),
      '@reports': path.join(ROOT, 'src/features/reports'),
      '@experiment-management-shared': path.join(ROOT, 'src/experiment-management-shared'),
    },
  },
};
