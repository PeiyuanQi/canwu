import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";

const legacyDocsRedirects = {
  "/docs": "/developer/",
  "/en/docs": "/en/developer/",
  "/docs/architecture": "/architecture/",
  "/en/docs/architecture": "/en/architecture/",
  "/docs/continuous-game-loop": "/tutorials/continuous-game-loop/",
  "/en/docs/continuous-game-loop": "/en/tutorials/continuous-game-loop/",
};

export default defineConfig({
  site: "https://canwu.org",
  output: "static",
  redirects: legacyDocsRedirects,
  vite: {
    build: {
      // Mermaid is split behind a dynamic import and downloaded only on pages with diagrams.
      chunkSizeWarningLimit: 675,
    },
  },
  integrations: [
    sitemap(),
    starlight({
      disable404Route: true,
      title: {
        "zh-CN": "参伍文档",
        en: "Canwu Documentation",
      },
      description: "Canwu tutorials, developer guides, architecture, and runnable examples for simulation developers.",
      locales: {
        root: { label: "简体中文", lang: "zh-CN" },
        en: { label: "English", lang: "en" },
      },
      favicon: "/brand/favicon.ico",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/PeiyuanQi/canwu",
        },
      ],
      editLink: {
        baseUrl: "https://github.com/PeiyuanQi/canwu/edit/main/website/",
      },
      components: {
        ThemeProvider: "./src/components/TutorialThemeProvider.astro",
      },
      customCss: ["./src/styles/starlight.css"],
      sidebar: [
        {
          label: "使用教程",
          translations: { en: "Tutorials" },
          items: [
            { slug: "tutorials" },
            { slug: "tutorials/move-army" },
            { slug: "tutorials/continuous-game-loop" },
            { slug: "tutorials/command-plugin" },
            { slug: "tutorials/phased-boundary" },
            { slug: "tutorials/routing-transport" },
            { slug: "tutorials/technology-diffusion" },
            {
              label: "案例",
              translations: { en: "Examples" },
              collapsed: true,
              items: [{ autogenerate: { directory: "tutorials/cases" } }],
            },
          ],
        },
        {
          label: "开发者指南",
          translations: { en: "Developer guide" },
          items: [{ autogenerate: { directory: "developer" } }],
        },
        {
          label: "架构",
          translations: { en: "Architecture" },
          items: [{ autogenerate: { directory: "architecture" } }],
        },
        {
          label: "参考",
          translations: { en: "Reference" },
          items: [{ slug: "reference/terminology" }],
        },
        {
          label: "GitHub 示例",
          translations: { en: "GitHub examples" },
          link: "https://github.com/PeiyuanQi/canwu/tree/main/crates/api/canwu-api/examples",
        },
      ],
    }),
  ],
});
