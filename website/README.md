# Canwu website

The public website for [canwu.org](https://canwu.org) is a static Astro site
deployed through GitHub Pages.

Chinese is the default language at `/`, `/showcase/`, and `/credits/`. English
uses matching routes under `/en/`, including `/en/showcase/` and
`/en/credits/`. Each page emits canonical and `hreflang` alternate metadata.

## Local development

Use Node.js 22.12 or newer and pnpm:

```text
pnpm install
pnpm dev
pnpm build
```

Astro telemetry is disabled by every package script through
`ASTRO_TELEMETRY_DISABLED=1`. The GitHub Pages workflow sets the same variable.

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
deploying with GitHub Actions. `public/CNAME` records the intended domain in the
site source but does not replace that setting.

## Credits and privacy

The public [credits page](https://canwu.org/credits/) lists the principal
frameworks, build tools, publishing services, visual sources, design references,
licenses, and trademark notices. The site uses no analytics, tracking pixels,
or third-party hosted fonts.

## License

The Canwu website source is licensed under the repository's
[Apache License 2.0](LICENSE). Third-party website tools and assets remain under
the licenses listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
