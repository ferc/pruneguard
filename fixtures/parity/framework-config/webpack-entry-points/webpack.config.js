const path = require('path');

module.exports = {
  entry: {
    main: './src/main.ts',
    vendor: './src/vendor.ts',
  },
  output: {
    path: path.resolve(__dirname, 'dist'),
    filename: '[name].bundle.js',
  },
};
