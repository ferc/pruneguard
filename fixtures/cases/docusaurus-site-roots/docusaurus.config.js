/** @type {import('@docusaurus/types').Config} */
const config = {
  title: "My Site",
  tagline: "A Docusaurus site",
  url: "https://example.com",
  baseUrl: "/",
  presets: [
    [
      "classic",
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: { sidebarPath: require.resolve("./sidebars.js") },
      }),
    ],
  ],
};

module.exports = config;
