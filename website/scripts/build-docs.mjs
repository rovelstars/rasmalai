import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static', 'data');
const compilerDir = join(root, '..', 'compiler');

mkdirSync(outDir, { recursive: true });

// Regenerates static/data/api.json from the compiler. docgen resolves
// @std/* from the global cargo cache or the registry (no embedded
// fallback), so seed the cache first: a cold cache plus an https-only
// registry would otherwise fail the docs build. Seeding is best-effort;
// docgen reports its own errors below.
try {
	execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', 'fetch-std'], {
		cwd: compilerDir,
		stdio: 'inherit'
	});
} catch {
	console.log('docs: stdlib seeding failed, docgen will use whatever the cache holds');
}

// The file is gitignored and served same-origin at /data/api.json, so a
// missing file is fine: pages render a "No API data yet" empty state
// instead of failing the build (an empty placeholder keeps prerendered
// fetches from 404ing). Refresh locally with:
//   npm run build:docs
try {
	execFileSync('cargo', ['run', '-q', '-p', 'docgen', '--', outDir], {
		cwd: compilerDir,
		stdio: 'inherit'
	});
	console.log('docs: api.json regenerated');
} catch {
	if (existsSync(join(outDir, 'api.json'))) {
		console.log('docs: cargo unavailable, keeping existing api.json');
	} else {
		writeFileSync(join(outDir, 'api.json'), '{"modules":[]}');
		console.log('docs: WARNING api.json could not be generated; wrote empty placeholder so the build stays green');
	}
}
