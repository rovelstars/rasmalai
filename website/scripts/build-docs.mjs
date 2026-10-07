import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static', 'data');
const compilerDir = join(root, '..', 'compiler');

mkdirSync(outDir, { recursive: true });

// Regenerates static/data/api.json from the compiler. Resolution order:
// 1. The live registry's docs bundle (same-origin data pattern): fastest,
//    hermetic, and always consistent with the published packages. This is
//    also what saves cargo-less build environments (Pages) - they reuse
//    the last good bundle instead of failing.
// 2. Local docgen (cargo): fresh snapshot from the working tree. Needs the
//    global stdlib cache first: docgen resolves @std/* from cache-or-
//    registry (no embedded fallback), so seed the cache before invoking
//    it. Seeding is best-effort; docgen reports its own errors.
// 3. Empty placeholder: pages render a "No API data yet" empty state
//    instead of failing the build (an absent file 404s prerendered
//    fetches, which fails `vite build`). A loud warning marks the
//    degradation. Refresh locally with:
//   npm run build:docs
// Automated builds (CI, Pages) set CI=true / CF_PAGES=1: they reuse the
// live bundle. Local runs regenerate from the tree so edits show up.
const autoBuild = process.env['CI'] === 'true' || process.env['CF_PAGES'] === '1';
const deployed = autoBuild ? fetchDeployedApiJson() : null;
if (deployed) {
	writeFileSync(join(outDir, 'api.json'), deployed);
	console.log('docs: api.json reused from the live registry');
} else {
	try {
		execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', 'fetch-std'], {
			cwd: compilerDir,
			stdio: 'inherit'
		});
	} catch {
		console.log('docs: stdlib seeding failed, docgen will use whatever the cache holds');
	}
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
}

function fetchDeployedApiJson() {
	const live = process.env['SITE_URL'] ?? 'https://rasmalai.rovelstars.com';
	try {
		const version = execFileSync(
			'curl',
			['-fsSL', '--max-time', '20', `${live.replace(/\/$/, '')}/data/version.json`],
			{ stdio: ['ignore', 'pipe', 'ignore'] }
		);
		const hash = JSON.parse(version.toString())?.version;
		if (typeof hash !== 'string' || !/^[0-9a-f]{8,64}$/.test(hash)) return null;
		const body = execFileSync(
			'curl',
			['-fsSL', '--max-time', '30', `${live.replace(/\/$/, '')}/data/${hash}/api.json`],
			{ stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 64 * 1024 * 1024 }
		);
		const snapshot = JSON.parse(body.toString());
		if (!snapshot || !Array.isArray(snapshot.modules) || snapshot.modules.length === 0) return null;
		return JSON.stringify(snapshot);
	} catch {
		return null;
	}
}
