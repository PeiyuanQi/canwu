import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

const siteDocsPrefix = "website/src/content/docs/";

function contentId(entry: string) {
  const normalizedEntry = entry.replaceAll("\\", "/");
  const relativeEntry = normalizedEntry.slice(siteDocsPrefix.length);

  return relativeEntry
    .replace(/\.(?:markdown|mdown|mkdn|mkd|mdwn|md|mdx)$/i, "")
    .replace(/\/(?:index|readme)$/i, "");
}

export const collections = {
  docs: defineCollection({
    loader: glob({
      base: "..",
      pattern: "website/src/content/docs/**/[^_]*.{markdown,mdown,mkdn,mkd,mdwn,md,mdx}",
      generateId: ({ entry }) => contentId(entry),
    }),
    schema: docsSchema(),
  }),
};
