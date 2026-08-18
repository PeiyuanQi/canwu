import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../../../", import.meta.url));
const repositoryDocsRoot = resolve(repositoryRoot, "docs");
const repositoryOverviewPath = resolve(repositoryDocsRoot, "README.md");
const githubBlobBase = "https://github.com/PeiyuanQi/canwu/blob/main/";
const githubRawBase = "https://raw.githubusercontent.com/PeiyuanQi/canwu/main/";

function isInside(parent, candidate) {
  const relativePath = relative(parent, candidate);
  return relativePath === "" || (!relativePath.startsWith("..") && !isAbsolute(relativePath));
}

function sourcePath(context) {
  if (!context.fileURL) return undefined;
  const path = fileURLToPath(context.fileURL);
  return isInside(repositoryDocsRoot, path) ? path : undefined;
}

function splitUrl(url) {
  const suffixIndex = url.search(/[?#]/);
  return suffixIndex === -1
    ? { path: url, suffix: "" }
    : { path: url.slice(0, suffixIndex), suffix: url.slice(suffixIndex) };
}

function encodePath(path) {
  return path.split("/").map(encodeURIComponent).join("/");
}

function repositoryDocRoute(filePath) {
  const relativePath = relative(repositoryDocsRoot, filePath)
    .replaceAll("\\", "/")
    .replace(/\.md$/i, "")
    .replace(/(?:^|\/)readme$/i, "");
  return relativePath ? `/en/docs/${relativePath}/` : "/en/docs/";
}

function rewriteUrl(url, currentSourcePath, nodeType) {
  if (
    !url ||
    url.startsWith("#") ||
    url.startsWith("/") ||
    url.startsWith("//") ||
    /^[a-z][a-z\d+.-]*:/i.test(url)
  ) {
    return url;
  }

  const { path, suffix } = splitUrl(url);
  const targetPath = resolve(dirname(currentSourcePath), decodeURI(path));

  if (isInside(repositoryDocsRoot, targetPath) && /\.md$/i.test(targetPath)) {
    return `${repositoryDocRoute(targetPath)}${suffix}`;
  }

  if (!isInside(repositoryRoot, targetPath)) return url;

  const repositoryPath = encodePath(relative(repositoryRoot, targetPath).replaceAll("\\", "/"));
  const base = nodeType === "image" ? githubRawBase : githubBlobBase;
  return `${base}${repositoryPath}${suffix}`;
}

function rewriteNodeUrl(node, context, nodeType) {
  const currentSourcePath = sourcePath(context);
  if (!currentSourcePath || !node.url) return;
  const rewrittenUrl = rewriteUrl(node.url, currentSourcePath, nodeType);
  if (rewrittenUrl !== node.url) context.setProperty(node, "url", rewrittenUrl);
}

function htmlSafeJson(value) {
  return JSON.stringify(value)
    .replaceAll("&", "\\u0026")
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e");
}

function repositoryDocsPlugin() {
  let overviewLeadReplaced = false;
  let diagramIndex = 0;

  return {
    name: "canwu-repository-docs",
    heading(node, context) {
      if (sourcePath(context) && node.depth === 1 && context.parent(node)?.type === "root") {
        context.removeNode(node);
      }
    },
    paragraph(node, context) {
      if (
        !overviewLeadReplaced &&
        sourcePath(context) === repositoryOverviewPath &&
        context.parent(node)?.type === "root"
      ) {
        overviewLeadReplaced = true;
        context.replaceNode(node, {
          type: "paragraph",
          children: [
            { type: "text", value: "Explore Canwu's architecture, engine contracts, integration guidance, community resources, and release references. Start with " },
            { type: "link", url: "/en/docs/architecture/", children: [{ type: "text", value: "Architecture" }] },
            { type: "text", value: " for the system boundary, or " },
            { type: "link", url: "/en/docs/continuous-game-loop/", children: [{ type: "text", value: "Continuous-time game loop" }] },
            { type: "text", value: " for host integration." },
          ],
        });
      }
    },
    code(node, context) {
      if (!sourcePath(context) || node.lang?.toLowerCase() !== "mermaid") return;

      diagramIndex += 1;
      const label = `Canwu documentation diagram ${diagramIndex}`;
      const definition = htmlSafeJson(node.value);
      context.replaceNode(node, {
        rawHtml: `<figure class="mermaid-figure">
  <figcaption class="sr-only">${label}. A text-source fallback is available below.</figcaption>
  <div class="mermaid" role="img" aria-label="${label}" aria-busy="true">Rendering diagram…</div>
  <details class="mermaid-source">
    <summary>View diagram source</summary>
    <pre><code class="mermaid-source-code">Diagram source loads with the preview.</code></pre>
  </details>
  <script type="application/json" class="mermaid-definition">${definition}</script>
  <noscript><p class="mermaid-noscript">JavaScript is required for the diagram preview; the Markdown source remains available from the page edit link.</p></noscript>
</figure>`,
      });
    },
    link(node, context) {
      rewriteNodeUrl(node, context, "link");
    },
    image(node, context) {
      rewriteNodeUrl(node, context, "image");
    },
    definition(node, context) {
      rewriteNodeUrl(node, context, "definition");
    },
  };
}

export default function repositoryDocsMarkdown() {
  return {
    name: "canwu-repository-docs-markdown",
    hooks: {
      "astro:config:setup": ({ config }) => {
        const processor = config.markdown.processor;
        if (processor.name !== "satteri" || !Array.isArray(processor.options.mdastPlugins)) {
          throw new Error("Canwu repository docs require Astro's Satteri Markdown processor.");
        }
        processor.options.mdastPlugins.push(repositoryDocsPlugin);
      },
    },
  };
}
