# Canwu website

The public website for [canwu.org](https://canwu.org) is a static Astro site
deployed through GitHub Pages. Its Markdown tutorial area uses Astro Starlight
inside the same build rather than a separate documentation application.

Chinese is the default language at `/`, `/showcase/`, and `/credits/`. English
uses matching routes under `/en/`, including `/en/showcase/` and
`/en/credits/`. Each page emits canonical and `hreflang` alternate metadata.
Documentation sources live in `src/content/docs/` (Chinese at the root, English
under `en/`). The Starlight sidebar in `astro.config.mjs` lists pages by slug in
seven sections, so a page's section does not have to match its URL:

- **Get started**: `/tutorials/` and the first-run tutorial `/tutorials/move-army/`.
- **Tutorials**: engine tutorials under `/tutorials/`, such as the continuous
  game loop, command plugin, and phased boundary.
- **Guides**: application tasks under `/developer/`.
- **Engine architecture**: `/architecture/` and its settlement, events,
  randomness, and model-ownership pages.
- **Domain systems**: the overview at `/architecture/systems/`, then one entry
  per system. Each entry starts with its design page under `/architecture/` and
  may add related walkthroughs from `/tutorials/`, guides from `/developer/`, or
  design records from `/proposals/`.
- **Case studies**: multi-system scenarios under `tutorials/cases/`, kept in one
  collapsed, auto-generated group so new scenarios do not lengthen the sidebar.
- **Reference**: the bilingual terminology page at `/reference/terminology/`
  and a link to the example source on GitHub.

English pages use the same paths under `/en/`. Adding a page outside
`tutorials/cases/` means adding its slug to the sidebar. Repository-level
design, maintainer, community, release, and legal documents remain under
`../docs/` and are deliberately not published as website documentation.

## Local development

Use Node.js 22.12 or newer and pnpm:

```text
pnpm install
pnpm dev
pnpm build
```

Astro telemetry is disabled by every package script through
`ASTRO_TELEMETRY_DISABLED=1`. The GitHub Pages workflow sets the same variable.
The tutorial UI stores only local presentation preferences such as color theme
and transient sidebar state; it does not add analytics or account tracking.

## GitHub Pages and custom domain

1. In the repository's **Settings → Pages**, select **GitHub Actions** as the
   build and deployment source.
2. Set the custom domain to `canwu.org`.
3. At the DNS provider, add these apex `A` records:

   ```text
   185.199.108.153
   185.199.109.153
   185.199.110.153
   185.199.111.153
   ```

4. Add these apex `AAAA` records when IPv6 is supported:

   ```text
   2606:50c0:8000::153
   2606:50c0:8001::153
   2606:50c0:8002::153
   2606:50c0:8003::153
   ```

5. Add a `CNAME` record for `www` pointing to `peiyuanqi.github.io`.
6. After DNS verification completes, enable **Enforce HTTPS** in GitHub Pages.

GitHub's repository Pages setting is authoritative for the custom domain when
deploying with GitHub Actions. `public/CNAME` records the same intended domain
in the site source.

## Credits and privacy

The public [credits page](https://canwu.org/credits/) lists the principal
frameworks, build tools, publishing services, visual sources, design references,
licenses, and trademark notices. The site serves static, analytics-free pages
with system-font stacks and zero third-party font requests.

## License

The Canwu website source is licensed under the repository's
[Apache License 2.0](LICENSE). Third-party website tools and assets remain under
the licenses listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
