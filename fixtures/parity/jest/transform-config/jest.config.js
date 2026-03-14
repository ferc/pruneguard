module.exports = {
  transform: {
    '\\.svg$': './src/transforms/svg-transform.js',
    '\\.tsx?$': 'ts-jest',
  },
};
