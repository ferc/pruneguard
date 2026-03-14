module.exports = {
  preset: 'ts-jest',
  moduleNameMapper: {
    '^@api/(.*)$': '<rootDir>/src/$1',
    '^@mocks/(.*)$': '<rootDir>/src/mocks/$1',
  },
};
