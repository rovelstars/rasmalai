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
  docs, search index, and benchmark data automatically).
- Build output directory: `.svelte-kit/cloudflare`.
- Environment: Node 22.

Benchmark data flow: the monthly `Monthly benchmarks` workflow re-runs
`python3 benches/harness/runner.py` and force-pushes the result to an
orphan `benchmarks` branch (single `benchmarks.json`, no history, nothing
lands on main). The homepage chart fetches that file in the visitor's
browser, so a build carries no snapshot and the published numbers are always
the latest measurement. Until the first run publishes a file, the chart
renders an empty state. Point it at another host by editing the
`benchmarksUrl` export in `src/lib/components/ParetoScatter.svelte`.
`scripts/sync-benchmarks.mjs` still writes a gitignored
`src/lib/benchmarks/pareto.json` for offline work and `prebuild` runs it
too; set `BENCHMARKS_URL` on the Pages project to have it fetch
`https://raw.githubusercontent.com/<owner>/<repo>/benchmarks/benchmarks.json`
instead of reading `benches/data/benchmarks.json`. `static/_headers` gives
content-hashed assets (`/_app/immutable/*`, fonts) a year of edge cache; HTML
and data revalidate every deploy.

Docs data flow: `prebuild` regenerates `api.json`, `search-index.json`,
and `cli-demos.json` from the current tree into gitignored
`static/data/` before every build, so each deploy serves data generated
from its own tree at same-origin `/data/*` — no branch, no snapshot on
main, no cross-origin fetch. Until the generators run (cargo unavailable),
pages render a "No API data yet" empty state and search reports that the
index is unavailable — the build still passes. Refresh local data with
`npm run build:docs` (needs cargo) and keep working offline.
