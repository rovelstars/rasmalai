// Stamps deploy-versioned copies of the prebuild docs data for immutable
// caching without any purge step. Reads static/data/{api,search-index,
// cli-demos}.json, copies each present file to static/data/<sha>/, and
// writes static/data/version.json {"version": "<sha>"}. Clients resolve
// payload URLs through version.json, so a deploy is fresh as soon as the
// pointer updates while every versioned payload caches immutably.
//
// <sha> is a content hash, not the commit id: rebuilding one commit with
// different bytes (the search index embeds live registry rows) mints a new
// sha instead of serving stale immutable bytes. Unversioned files stay in
// place as the fallback for older bundles and local dev. Stale version
// dirs from earlier local builds are pruned so deploys stay lean.
// Missing files are fine (cargo unavailable): pages render empty states.
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dataDir = join(root, 'static', 'data');
const FILES = ['api.json', 'search-index.json', 'cli-demos.json'];
const SHA_RE = /^[0-9a-f]{16}$/;

const present = FILES.filter((f) => existsSync(join(dataDir, f)));
if (present.length === 0) {
	console.log('version-data: no data files, skipping (pages render empty states)');
	process.exit(0);
}

const hash = createHash('sha256');
for (const f of present) {
	hash.update(f);
	hash.update(readFileSync(join(dataDir, f)));
}
const sha = hash.digest('hex').slice(0, 16);

const versionDir = join(dataDir, sha);
mkdirSync(versionDir, { recursive: true });
for (const f of present) copyFileSync(join(dataDir, f), join(versionDir, f));
writeFileSync(join(dataDir, 'version.json'), JSON.stringify({ version: sha }) + '\n');

for (const entry of readdirSync(dataDir)) {
	const full = join(dataDir, entry);
	if (entry !== sha && SHA_RE.test(entry) && existsSync(full) && statSync(full).isDirectory()) {
		rmSync(full, { recursive: true, force: true });
	}
}
console.log(`version-data: ${present.length} files versioned under /data/${sha}/`);
