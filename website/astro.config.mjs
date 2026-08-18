import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://canwu.org",
  output: "static",
  integrations: [
    sitemap(),
    starlight({
      disable404Route: true,
      title: {
        "zh-CN": "参伍使用教程",
        en: "Canwu Tutorials",
      },
      description: "Canwu public API tutorials and runnable examples for game and simulation developers.",
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
          items: [{ autogenerate: { directory: "tutorials" } }],
        },
        {
          label: "GitHub 示例",
          translations: { en: "GitHub examples" },
          link: "https://github.com/PeiyuanQi/canwu/tree/main/crates/canwu-api/examples",
        },
      ],
    }),
  ],
});
