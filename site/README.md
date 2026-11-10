# irit documentation site

Astro + Starlight site for https://dzulfikar08.github.io/irit, built from `../docs/`.

```sh
npm ci
npm run dev     # sync docs/, then serve at http://localhost:4321/irit/
npm run build   # sync docs/, build to dist/, fail on broken internal links or anchors
```

- **Content lives in `../docs/`.** `scripts/sync-docs.mjs` copies it into `src/content/docs/`
  (gitignored) before every `dev` and `build`: the first `# Heading` becomes the page title,
  links to other docs pages become site URLs, links to other repository files go to GitHub.
  Restart `npm run dev` to pick up edits made in `docs/`.
- **Sidebar** comes from `../docs/SUMMARY.md` (see `site.config.mjs`). A page not listed there
  still appears: in the group whose pages share its folder, otherwise under "More".
- **Landing page** is `src/content/docs/index.mdx`, the only committed page here. Its numbers
  must match `docs/benchmarks.md` and the README; `src/components/Measured.astro` holds the chart.
- **Theme** is `src/styles/theme.css`.

Deployed by `.github/workflows/pages.yml` on pushes to `main` that touch `docs/`, `site/` or the
workflow. Node version: `.nvmrc`.
