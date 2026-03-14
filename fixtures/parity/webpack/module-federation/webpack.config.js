const { ModuleFederationPlugin } = require('webpack').container;

module.exports = {
  plugins: [
    new ModuleFederationPlugin({
      name: 'shared_ui',
      exposes: {
        './Button': './src/Button',
        './Header': './src/Header',
      },
    }),
  ],
};
