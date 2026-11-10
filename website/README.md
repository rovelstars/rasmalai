# sv

Everything you need to build a Svelte project, powered by [`sv`](https://github.com/sveltejs/cli).

## Creating a project

If you're seeing this, you've probably already done this step. Congrats!

```sh
# create a new project
npx sv create my-app
```

To recreate this project with the same configuration:

```sh
# recreate this project
npx sv@0.17.0 create --template minimal --types ts --no-install website
```

## Developing

Once you've created a project and installed dependencies with `npm install` (or `pnpm install` or `yarn`), start a development server:

```sh
npm run dev

# or start the server and open the app in a new browser tab
npm run dev -- --open
```

### Playground stdlib origin

In production the playground reads `@std/*` from the same origin. Under
`npm run dev`, the local server serves `compiler/stdlib/src` from the
checkout and the browser skips its stdlib cache, so a local edit is
visible on the next run. No registry or published package is involved.

To test a published registry instead, set the override explicitly:

```sh
VITE_RNX_REGISTRY=https://rasmalai.rovelstars.com npm run dev
```

The override has highest priority and disables the local checkout path.
It also disables the production fallback. Production builds leave it
unset and use the page origin.

## Building

To create a production version of your app:

```sh
npm run build
```

You can preview the production build with `npm run preview`.

> To deploy your app, you may need to install an [adapter](https://svelte.dev/docs/kit/adapters) for your target environment.

## Deployment (Cloudflare Pages)

The site builds with `@sveltejs/adapter-cloudflare` and deploys as a
static Pages project (free tier is enough: ~500 builds/month, automatic
caching). One-time setup in the Cloudflare dashboard:

- Connect the GitHub repository, set the project root to `website`.
- Build command: `npm run build:wasm && npm run build`
  (`build:wasm` compiles the playground engine; `prebuild` then syncs
  docs and the search index automatically).
- Build output directory: `.svelte-kit/cloudflare`.
- Environment: Node 22.

Benchmark data flow: the monthly `Monthly benchmarks` workflow re-runs
`python3 benches/harness/runner.py` and POSTs the result to
`/api/benchmarks`, which stores it as one row in the Turso `benchmark_runs`
table. The homepage chart fetches `/api/benchmarks` in the visitor's
browser from the Cloudflare Pages Function, which caches the response for an
hour at the edge, so no snapshot is committed to the repository. Until the
first run ingests a snapshot, the chart renders an empty state. The fetch
lives in `src/lib/components/ParetoScatter.svelte`.
`static/_headers` gives
content-hashed assets (`/_app/immutable/*`, fonts) a year of edge cache; HTML
and data revalidate every deploy.

Docs data flow: `prebuild` regenerates `api.json`, `search-index.json`,
and `cli-demos.json` from the current tree into
`static/data/` before every build, then `scripts/version-data.mjs` copies
them under `static/data/<sha>/` (content hash) and writes a
`static/data/version.json` pointer, so each deploy serves immutable
per-deploy payloads at same-origin `/data/<sha>/*` — no branch, no
snapshot on main, no cross-origin fetch, no purge step. Clients resolve
payload URLs through `version.json` (60s cache) and fall back to
unversioned `/data/*` when the pointer is missing. Until the generators
run (cargo unavailable), pages render a "No API data yet" empty state and
search reports that the index is unavailable — the build still passes.
Refresh local data with `npm run build:docs` (needs cargo) and keep
working offline.
