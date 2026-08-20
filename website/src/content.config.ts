import { readFileSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineCollection } from "astro:content";
import { glob, type Loader } from "astro/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

const repositoryRoot = fileURLToPath(new URL("../..", import.meta.url));
const repositoryDocsRoot = resolve(repositoryRoot, "docs");
const siteDocsPrefix = "website/src/content/docs/";
const repositoryDocsPrefix = "docs/";

function contentId(entry: string) {
  const normalizedEntry = entry.replaceAll("\\", "/");
  const relativeEntry = normalizedEntry.startsWith(siteDocsPrefix)
    ? normalizedEntry.slice(siteDocsPrefix.length)
    : `en/docs/${normalizedEntry.slice(repositoryDocsPrefix.length)}`;

  return relativeEntry
    .replace(/\.(?:markdown|mdown|mkdn|mkd|mdwn|md|mdx)$/i, "")
    .replace(/\/(?:index|readme)$/i, "");
}

function isRepositoryDoc(filePath: string | undefined) {
  if (!filePath) return false;
  const relativePath = relative(repositoryDocsRoot, resolve(filePath));
  return relativePath !== "" && !relativePath.startsWith("..") && !isAbsolute(relativePath);
}

function repositoryDocTitle(filePath: string) {
  const source = readFileSync(filePath, "utf8");
  const firstLine = source.replace(/^\uFEFF/, "").split(/\r?\n/, 1)[0];
  const heading = firstLine.match(/^#\s+(.+?)(?:\s+#+)?\s*$/)?.[1];

  if (!heading) {
    throw new Error(`Repository document ${filePath} must start with a level-one heading.`);
  }

  return heading;
}

function canwuDocsLoader(): Loader {
  const loader = glob({
    base: "..",
    pattern: [
      "website/src/content/docs/**/[^_]*.{markdown,mdown,mkdn,mkd,mdwn,md,mdx}",
      "docs/**/[^_]*.md",
    ],
    generateId: ({ entry }) => contentId(entry),
  });

  return {
    name: "canwu-docs-loader",
    async load(context) {
      await loader.load({
        ...context,
        async parseData(options) {
          if (!isRepositoryDoc(options.filePath)) {
            return context.parseData(options);
          }

          const repositoryPath = relative(repositoryRoot, options.filePath!)
            .replaceAll("\\", "/");
          const isOverview = repositoryPath === "docs/README.md";

          return context.parseData({
            ...options,
            data: {
              ...options.data,
              title: isOverview ? "Overview" : repositoryDocTitle(options.filePath!),
              description: isOverview
                ? "Explore Canwu architecture, engine contracts, integration guides, community resources, and release references."
                : options.data.description,
              ...(isOverview ? { template: "doc" } : {}),
              editUrl: `https://github.com/PeiyuanQi/canwu/edit/main/${repositoryPath}`,
            },
          });
        },
      });
    },
  };
}

export const collections = {
  docs: defineCollection({ loader: canwuDocsLoader(), schema: docsSchema() }),
};
