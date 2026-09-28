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
  "/reference/ming-fiscal": "/tutorials/cases/ming-fiscal/",
  "/en/reference/ming-fiscal": "/en/tutorials/cases/ming-fiscal/",
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
      // Sections, in reading order: first run, engine tutorials, application
      // guides, engine design, optional domain systems, multi-system case
      // studies, and reference. Entries are listed by slug so a page's URL
      // does not have to match the section that lists it.
      sidebar: [
        {
          label: "快速开始",
          translations: { en: "Get started" },
          items: [
            { slug: "tutorials", label: "概览", translations: { en: "Overview" } },
            {
              slug: "tutorials/move-army",
              label: "第一次运行：移动军队",
              translations: { en: "First run: move an army" },
            },
          ],
        },
        {
          label: "教程",
          translations: { en: "Tutorials" },
          items: [
            { slug: "tutorials/continuous-game-loop" },
            { slug: "tutorials/command-plugin" },
            { slug: "tutorials/phased-boundary" },
          ],
        },
        {
          label: "开发指南",
          translations: { en: "Guides" },
          items: [
            { slug: "developer", label: "概览", translations: { en: "Overview" } },
            { slug: "developer/integration" },
            { slug: "developer/reading-state" },
            { slug: "developer/persistence" },
            { slug: "developer/extensions" },
          ],
        },
        {
          label: "引擎架构",
          translations: { en: "Engine architecture" },
          items: [
            { slug: "architecture", label: "架构总览", translations: { en: "Overview" } },
            { slug: "architecture/settlement" },
            { slug: "architecture/events" },
            { slug: "architecture/randomness" },
            { slug: "architecture/model-ownership" },
          ],
        },
        {
          label: "领域系统",
          translations: { en: "Domain systems" },
          items: [
            { slug: "architecture/systems", label: "概览", translations: { en: "Overview" } },
            {
              slug: "architecture/knowledge-information",
              label: "知识与信息",
              translations: { en: "Knowledge and information" },
            },
            { slug: "architecture/decisions", label: "决策", translations: { en: "Decisions" } },
            {
              label: "资源与生产",
              translations: { en: "Resources and production" },
              collapsed: true,
              items: [
                { slug: "architecture/resources-production", label: "设计", translations: { en: "Design" } },
                { slug: "developer/production-economy", label: "职责划分", translations: { en: "Who owns what" } },
                {
                  slug: "proposals/production-economy-mechanism",
                  label: "设计决策记录",
                  translations: { en: "Design decision record" },
                },
              ],
            },
            {
              label: "路线、运输与移动",
              translations: { en: "Routing, transport, and movement" },
              collapsed: true,
              items: [
                { slug: "architecture/routing-movement", label: "设计", translations: { en: "Design" } },
                {
                  slug: "tutorials/routing-transport",
                  label: "实践：路线规划与送信",
                  translations: { en: "Walkthrough: route planning and delivery" },
                },
              ],
            },
            {
              label: "军事",
              translations: { en: "Military" },
              collapsed: true,
              items: [
                { slug: "architecture/military", label: "设计", translations: { en: "Design" } },
                {
                  slug: "tutorials/military-domain",
                  label: "实践：军事扩展",
                  translations: { en: "Walkthrough: military extension" },
                },
              ],
            },
            {
              label: "技术",
              translations: { en: "Technology" },
              collapsed: true,
              items: [
                { slug: "architecture/technology", label: "设计", translations: { en: "Design" } },
                {
                  slug: "tutorials/technology-diffusion",
                  label: "实践：技术传播",
                  translations: { en: "Walkthrough: technology diffusion" },
                },
              ],
            },
            { slug: "architecture/fiscal", label: "财政制度", translations: { en: "Fiscal institutions" } },
            {
              slug: "architecture/culture-law",
              label: "社会、文化与法律",
              translations: { en: "Society, culture, and law" },
            },
          ],
        },
        {
          label: "案例",
          translations: { en: "Case studies" },
          collapsed: true,
          items: [{ autogenerate: { directory: "tutorials/cases" } }],
        },
        {
          label: "参考",
          translations: { en: "Reference" },
          items: [
            { slug: "reference/terminology" },
            {
              label: "canwu-api 示例源码",
              translations: { en: "canwu-api examples on GitHub" },
              link: "https://github.com/PeiyuanQi/canwu/tree/main/crates/api/canwu-api/examples",
            },
          ],
        },
      ],
    }),
  ],
});
