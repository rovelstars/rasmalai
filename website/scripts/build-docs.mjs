import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync, mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
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
// 2. Local docgen (cargo): fresh snapshot from the working tree. Docgen
//    self-seeds @std/* over HTTPS, so no CLI or LLVM is needed here;
//    docgen reports its own errors.
// 3. Release binary docgen (no cargo/LLVM needed): the published rnx
//    release still carries embedded stdlib, so `doc --stdlib` works fully
//    offline. This is what saves LLVM-less build environments (Pages).
//    Output shape is validated before use; a shape change in the tree's
//    docgen fails closed to step 4 rather than shipping stale docs.
// 4. Empty placeholder: pages render a "No API data yet" empty state
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
} else if (!tryCargoDocgen()) {
	if (!tryReleaseDocgen()) {
		if (existsSync(join(outDir, 'api.json'))) {
			console.log('docs: cargo unavailable, keeping existing api.json');
		} else {
			writeFileSync(join(outDir, 'api.json'), '{"modules":[]}');
			console.log('docs: WARNING api.json could not be generated; wrote empty placeholder so the build stays green');
		}
	}
}

function tryCargoDocgen() {
	try {
		execFileSync('cargo', ['run', '-q', '-p', 'docgen', '--', outDir], {
			cwd: compilerDir,
			stdio: 'inherit'
		});
	} catch {
		return false;
	}
	if (!validApiJson(join(outDir, 'api.json'))) {
		console.log('docs: cargo docgen output failed validation, discarding');
		return false;
	}
	console.log('docs: api.json regenerated with cargo docgen');
	return true;
}

// Fetch a prebuilt rnx release (pinned per platform) and document its
// embedded stdlib. No cargo, no LLVM, no network beyond the download.
function tryReleaseDocgen() {
	const target =
		process.platform === 'linux' && process.arch === 'x64'
			? 'x86_64-linux'
			: process.platform === 'darwin' && process.arch === 'arm64'
				? 'aarch64-macos'
				: process.platform === 'win32' && process.arch === 'x64'
					? 'x86_64-windows'
					: null;
	if (!target) {
		console.log(`docs: no release binary for ${process.platform}/${process.arch}, skipping`);
		return false;
	}
	const dir = mkdtempSync(join(tmpdir(), 'rnx-rel-'));
	try {
		try {
			execFileSync(
				'curl',
				[
					'-fsSL',
					'--max-time',
					'180',
					'-o',
					join(dir, 'rnx.tar.gz'),
					`https://github.com/rovelstars/rasmalai/releases/latest/download/rnx-${target}.tar.gz`
				],
				{ stdio: ['ignore', 'pipe', 'ignore'] }
			);
		} catch (e) {
			console.log(`docs: release download failed: ${errorMessage(e)}`);
			return false;
		}
		try {
			execFileSync('tar', ['xzf', join(dir, 'rnx.tar.gz'), '-C', dir], { stdio: ['ignore', 'pipe', 'ignore'] });
		} catch (e) {
			console.log(`docs: release extract failed: ${errorMessage(e)}`);
			return false;
		}
		const bin = join(dir, `rnx-${target}`, 'bin', process.platform === 'win32' ? 'rnx.exe' : 'rnx');
		if (!existsSync(bin)) {
			console.log('docs: release archive has no rnx binary, skipping');
			return false;
		}
		// Newer binaries ship no embedded stdlib and resolve @std/* from
		// cache-or-registry instead: seed first (warn-only), so both eras
		// of release binary work here.
		try {
			execFileSync(bin, ['fetch-std'], { stdio: ['ignore', 'pipe', 'ignore'] });
		} catch {
			console.log('docs: release fetch-std failed, relying on embedded stdlib if present');
		}
		try {
			execFileSync(bin, ['doc', '--json', '--stdlib', '--out-dir', outDir], {
				stdio: ['ignore', 'pipe', 'ignore']
			});
		} catch (e) {
			console.log(`docs: release doc failed: ${errorMessage(e)}`);
			return false;
		}
		if (!validApiJson(join(outDir, 'api.json'))) {
			console.log('docs: release docgen output failed validation, discarding');
			return false;
		}
		console.log('docs: api.json generated with the release binary');
		return true;
	} catch (e) {
		console.log(`docs: release path failed: ${errorMessage(e)}`);
		return false;
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
}

function errorMessage(e) {
	return e instanceof Error ? (e.message.split('\n')[0] ?? String(e)).slice(0, 200) : String(e).slice(0, 200);
}

function validApiJson(path) {
	try {
		const snapshot = JSON.parse(readFileSync(path, 'utf8'));
		if (!snapshot || !Array.isArray(snapshot.modules) || snapshot.modules.length === 0) return false;
		const first = snapshot.modules[0];
		return (
			typeof first?.name === 'string' &&
			Array.isArray(first?.functions) &&
			Array.isArray(first?.classes)
		);
	} catch {
		return false;
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
