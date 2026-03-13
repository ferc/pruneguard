import { defineConfig } from "vitepress";

export default defineConfig({
  title: "My Docs",
  description: "Documentation site",
  themeConfig: {
    nav: [
      { text: "Home", link: "/" },
    ],
    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Introduction", link: "/guide/" },
        ],
      },
    ],
  },
});
