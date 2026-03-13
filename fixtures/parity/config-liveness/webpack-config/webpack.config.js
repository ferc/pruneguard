const path = require('path');

module.exports = {
  resolve: {
    alias: {
      '@helpers': path.resolve(__dirname, 'src/helpers'),
    },
  },
};
