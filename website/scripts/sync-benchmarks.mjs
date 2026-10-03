#!/usr/bin/env node
// Sync measured results from the unified benchmark harness into a local
// snapshot for development. The homepage Pareto chart fetches published
// data at runtime and renders an empty state when none exists, so this
// file is only a local convenience and is gitignored.
//
// Usage:
//   node scripts/sync-benchmarks.mjs
//   node scripts/sync-benchmarks.mjs /path/to/benchmarks.json
//
// Default source is ../../benches/data/benchmarks.json (written by
// `python3 benches/harness/runner.py`; that file is gitignored, generate
// it locally before syncing). When BENCHMARKS_URL is set, the JSON is
// fetched over HTTPS instead - the Pages build sets it to the orphan
// benchmarks branch:
//   https://raw.githubusercontent.com/<owner>/<repo>/benchmarks/benchmarks.json
// Validates the schema, then writes src/lib/benchmarks/pareto.json.
//
// A result row is one (language, mode) pair: rnx, c, rust, and dart each
// contribute a "dev" and a "rel" row; go, node, and java have one honest
// configuration apiece. `build_ms` is that row's own build step, so the
// chart's y axis is per-point rather than a separate dev column.

import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const srcArg = process.argv[2];
const srcUrl = !srcArg ? process.env.BENCHMARKS_URL : undefined;
const srcPath = srcArg ?? join(root, '..', 'benches', 'data', 'benchmarks.json');
const outPath = join(root, 'src', 'lib', 'benchmarks', 'pareto.json');

let text;
if (srcUrl) {
	const res = await fetch(srcUrl);
	if (!res.ok) throw new Error(`benchmarks fetch failed: ${res.status} ${srcUrl}`);
	text = await res.text();
} else {
	if (!existsSync(srcPath)) {
		console.log('benchmarks: no local data, skipping (monthly workflow publishes it)');
		process.exit(0);
	}
	text = readFileSync(srcPath, 'utf8');
}
const data = JSON.parse(text);

const isNum = (v) => typeof v === 'number' && Number.isFinite(v) && v >= 0;
if (!data.system?.cpu || !data.system?.os || !data.system?.date) {
	throw new Error('benchmarks.json: missing system.cpu/os/date');
}
const ids = Object.keys(data.benchmarks ?? {});
if (ids.length === 0) throw new Error('benchmarks.json: no benchmarks');
for (const id of ids) {
	const b = data.benchmarks[id];
	if (!b.name || !b.category || !b.description || !Array.isArray(b.results) || b.results.length === 0) {
		throw new Error(`benchmarks.json: benchmark ${id} missing name/category/description/results`);
	}
	const labels = new Set();
	for (const r of b.results) {
		const where = `${id}/${r.label ?? r.lang}`;
		if (typeof r.lang !== 'string' || typeof r.mode !== 'string') {
			throw new Error(`benchmarks.json: ${where} missing lang/mode`);
		}
		if (r.mode !== 'dev' && r.mode !== 'rel') {
			throw new Error(`benchmarks.json: ${where} mode must be dev or rel`);
		}
		if (r.label !== `${r.lang} ${r.mode}`) {
			throw new Error(`benchmarks.json: ${where} label must read "<lang> <mode>"`);
		}
		if (labels.has(r.label)) throw new Error(`benchmarks.json: ${where} duplicate label`);
		labels.add(r.label);
		if (!isNum(r.runtime_ms) || !isNum(r.peak_rss_mb) || !isNum(r.build_ms)) {
			throw new Error(`benchmarks.json: ${where} has invalid numbers`);
		}
		if (typeof r.build !== 'string' || !r.build || typeof r.run !== 'string' || !r.run) {
			throw new Error(`benchmarks.json: ${where} missing build/run provenance`);
		}
		if (typeof r.artifact !== 'boolean') {
			throw new Error(`benchmarks.json: ${where} missing artifact flag`);
		}
		if (r.artifact && r.build_ms === 0) {
			throw new Error(`benchmarks.json: ${where} builds an artifact but build_ms is 0`);
		}
	}
}

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, JSON.stringify(data, null, 2) + '\n');
console.log(`synced ${ids.length} benchmarks -> ${outPath}`);
