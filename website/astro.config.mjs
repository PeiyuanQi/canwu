import { defineConfig } from "astro/config";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";
import repositoryDocsMarkdown from "./src/plugins/repository-docs-markdown.mjs";

const repositoryDocRoutes = [
  "",
  "architecture",
  "continuous-game-loop",
  "end-state",
  "engine-conformance",
  "versioning",
  "community/branding",
  "community/sponsors",
  "legal/third-party-licenses",
  "legal/third-party-notices-extra",
];

const repositoryDocRedirects = Object.fromEntries(
  repositoryDocRoutes.map((route) => [
    route ? `/docs/${route}` : "/docs",
    route ? `/en/docs/${route}/` : "/en/docs/",
  ]),
);

export default defineConfig({
  site: "https://canwu.org",
  output: "static",
  redirects: repositoryDocRedirects,
  vite: {
    build: {
      // Mermaid is split behind a dynamic import and downloaded only on pages with diagrams.
      chunkSizeWarningLimit: 675,
    },
  },
  integrations: [
    sitemap(),
    repositoryDocsMarkdown(),
    starlight({
      disable404Route: true,
      title: {
        "zh-CN": "参伍文档",
        en: "Canwu Documentation",
      },
      description: "Canwu tutorials, architecture, integration guides, and project reference documentation.",
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
        LanguageSelect: "./src/components/LanguageSelect.astro",
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
          label: "架构",
          translations: { en: "Architecture" },
          items: [{ autogenerate: { directory: "architecture" } }],
        },
        {
          label: "项目文档（英文）",
          translations: { en: "Project docs" },
          items: [
            { label: "文档索引", translations: { en: "Overview" }, link: "/docs/" },
            { label: "完整架构规范", translations: { en: "Architecture reference" }, link: "/docs/architecture/" },
            { label: "最终设计", translations: { en: "End-state design" }, link: "/docs/end-state/" },
            { label: "引擎一致性", translations: { en: "Engine conformance" }, link: "/docs/engine-conformance/" },
            { label: "版本与兼容性", translations: { en: "Versioning" }, link: "/docs/versioning/" },
            { label: "连续时间循环", translations: { en: "Continuous-time loop" }, link: "/docs/continuous-game-loop/" },
            {
              label: "社区",
              translations: { en: "Community" },
              items: [
                { label: "品牌指南", translations: { en: "Branding" }, link: "/docs/community/branding/" },
                { label: "赞助者", translations: { en: "Sponsors" }, link: "/docs/community/sponsors/" },
              ],
            },
            {
              label: "法律与发布",
              translations: { en: "Legal and release" },
              items: [
                { label: "第三方许可证", translations: { en: "Third-party licenses" }, link: "/docs/legal/third-party-licenses/" },
                { label: "补充声明", translations: { en: "Additional notices" }, link: "/docs/legal/third-party-notices-extra/" },
              ],
            },
          ],
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
