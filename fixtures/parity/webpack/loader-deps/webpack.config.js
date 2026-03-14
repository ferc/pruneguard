module.exports = {
  entry: './src/main.ts',
  module: {
    rules: [
      { test: /\.tsx?$/, use: 'ts-loader' },
      { test: /\.jsx?$/, use: 'babel-loader' },
      { test: /\.css$/, use: 'css-loader' },
    ],
  },
};
