#!/usr/bin/env node
// Publishes every @std/* module as a full registry package through the
// protocol-0 source-tree frame (the server canonicalizes, redacts, chunks
// and analyses the upload), so chunks-fetch, jsdoc, and the code browser
// work for stdlib. Idempotent: already-published versions come back 409
// and are skipped. Re-run after every release; new versions publish, old
// ones skip. Every staged package declares engine >= the publishing CLI,
// so older toolchains (which cannot parse the current stdlib sources)
// keep resolving the older compatible line instead of installing bytes
// they cannot compile.
//
//   SITE_URL=https://... PUBLISH_TOKEN=... node scripts/publish-std.mjs
// Uses `target/release/rnx` when present, otherwise `cargo run -q -p cli`.
import { readFileSync, readdirSync, mkdirSync, rmSync, cpSync, writeFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(root, '..');
const stdDir = join(repo, 'compiler', 'stdlib', 'src');
const stage = join(root, '.std-publish');

const SITE = (process.env['SITE_URL'] ?? 'https://rasmalai.rovelstars.com').replace(/\/$/, '');
const TOKEN = process.env['PUBLISH_TOKEN'];
if (!TOKEN) {
	console.error('publish-std: missing PUBLISH_TOKEN');
	process.exit(1);
}
const rnxBin = join(repo, 'target', 'release', 'rnx');
const rnx = (args, cwd) => {
	if (existsSync(rnxBin)) return execFileSync(rnxBin, args, { cwd, stdio: 'pipe' });
	return execFileSync(
		'cargo',
		['run', '-q', '--manifest-path', join(repo, 'compiler', 'cli', 'Cargo.toml'), '--', ...args],
		{ cwd, stdio: 'pipe' },
	);
};

const cliVersion = readFileSync(join(repo, 'compiler', 'cli', 'Cargo.toml'), 'utf8').match(
	/^version = "(.*)"$/m
)?.[1];
if (!cliVersion) throw new Error('cannot read cli version');
// The token travels in the environment, never in argv: exec failures
// embed the command line, and argv would leak the secret into logs.
process.env['RNX_TOKEN'] = TOKEN;

const found = [];
for (const e of readdirSync(stdDir, { withFileTypes: true })) {
	if (e.isDirectory() && existsSync(join(stdDir, e.name, 'lib.rnx'))) found.push(e.name);
}
// net/http.rnx is a submodule file, not a package: it ships inside @std/net.
const SUBMODULES = { net: ['http.rnx'] };

// Publish dependencies before dependents. The server analyses each upload
// against the versions already in the registry, so a prelude published before
// its @std/time would be checked against the older time and fail on an export
// that only the new source has.
function importDeps(name) {
	let src = readFileSync(join(stdDir, name, 'lib.rnx'), 'utf8');
	for (const sub of SUBMODULES[name] ?? []) {
		src += '\n' + readFileSync(join(stdDir, name, sub), 'utf8');
	}
	src = src.replace(/\/\*[\s\S]*?\*\//g, '');
	src = src.split('\n').filter((line) => !line.trimStart().startsWith('//')).join('\n');
	return new Set(
		[...src.matchAll(/from\s+"@std\/([A-Za-z][A-Za-z0-9_]*)/g)]
			.map((m) => m[1])
			.filter((dep) => dep !== name)
	);
}

function publishOrder(names) {
	const known = new Set(names);
	const deps = new Map(names.map((name) => [name, [...importDeps(name)].filter((d) => known.has(d))]));
	const done = new Set();
	const out = [];
	const visit = (name) => {
		if (done.has(name)) return;
		done.add(name);
		for (const dep of [...deps.get(name)].sort()) visit(dep);
		out.push(name);
	};
	for (const name of [...names].sort()) visit(name);
	return out;
}

const modules = publishOrder(found);

// Declared permissions per std module: the union of the analyzer verdict
// (`rnx audit`, the same predicate as the server capability-coverage gate)
// and the textual backstop scan (website/src/lib/server/permissions.ts)
// over shipped sources. Only modules whose shipped code observably uses a
// capability sink declare one; everything else publishes with no
// permissions key (pure).
const PERMISSIONS = {
	env: [{ perm: 'env:read:*', reason: 'Env.get reads caller-provided keys' }],
	fs: [
		{ perm: 'fs:read:*', reason: 'File reads and directory scans take caller-provided paths' },
		{ perm: 'fs:write:*', reason: 'File writes, creates, and removals take caller-provided paths' },
		{ perm: 'fs:delegated', reason: 'path helpers pass caller paths through to fs sinks' },
		'term:read',
		'term:write',
		{ perm: 'unsafe:raw_memory', reason: 'mmap is backed by a raw address' }
	],
	io: [
		'term:read',
		'term:write',
		{ perm: 'env:read:NO_COLOR', reason: 'color detection reads NO_COLOR' },
		{ perm: 'env:read:COLORTERM', reason: 'color detection reads COLORTERM' },
		{ perm: 'env:read:TERM', reason: 'color detection reads TERM' },
		{ perm: 'fs:read:*', reason: 'stdin and file handles read through @std/fs' },
		{ perm: 'fs:write:*', reason: 'stdout and file handles write through @std/fs' },
		{ perm: 'fs:delegated', reason: 'file-handle helpers pass caller paths through to fs sinks' },
		{ perm: 'unsafe:raw_memory', reason: 'wrapped File handles include raw-memory mappings' }
	],
	process: [
		{ perm: 'env:read:*', reason: 'flattenEnv reads caller-provided keys' },
		{ perm: 'sys:exec:*', reason: 'Process.spawn and Process.run execute caller-provided binaries' }
	],
	log: [
		'term:write',
		{ perm: 'env:read:RNX_LOG', reason: 'RNX_LOG selects the log threshold' },
		{ perm: 'fs:read:*', reason: 'stderr handles read through @std/fs' },
		{ perm: 'fs:write:*', reason: 'stderr handles write through @std/fs' },
		{ perm: 'unsafe:raw_memory', reason: 'stderr file handles include raw-memory mappings' }
	],
	prelude: [
		'term:write',
		{ perm: 'env:read:*', reason: 're-exported io and log read environment keys' },
		{ perm: 'fs:read:*', reason: 're-exported io handles read through @std/fs' },
		{ perm: 'fs:write:*', reason: 're-exported io handles write through @std/fs' },
		{ perm: 'unsafe:raw_memory', reason: 're-exported io handles include raw-memory mappings' }
	],
	app: [
		'term:write',
		'fs:delegated',
		{ perm: 'fs:read:*', reason: 'AppFiles reads caller-provided paths' },
		{ perm: 'fs:write:*', reason: 'AppFiles and AppBundle write caller-provided paths' },
		{ perm: 'unsafe:ffi', reason: 'app wraps the native host and text ABIs' },
		{ perm: 'unsafe:raw_memory', reason: 'frame and packet buffers cross the C ABI' },
		{ perm: 'env:read:RNX_SURFACE_SOCKET', reason: 'selects the surface socket transport' },
		{ perm: 'env:read:RNX_UI_SCRIPT', reason: 'selects the headless UI script' },
		{ perm: 'env:read:RNX_INSPECT', reason: 'enables the node inspector overlay' }
	],
	uefi: ['term:write'],
	wasi: ['term:write']
};

function renderPermissions(name, manifest) {
	const decls = manifest?.permissions ?? PERMISSIONS[name];
	if (!decls || decls.length === 0) return '';
	const items = decls.map((d) =>
		typeof d === 'string' ? JSON.stringify(d) : `{ perm: ${JSON.stringify(d.perm)}, reason: ${JSON.stringify(d.reason)} }`
	);
	return `,\n    permissions: [${items.join(', ')}]`;
}

// Native and system dependencies survive; path dependencies point at the
// checkout and cannot travel with the published package.
function renderDependencies(manifest) {
	const deps = manifest?.dependencies ?? {};
	const keep = Object.entries(deps).filter(([, v]) => !(v && typeof v === 'object' && 'path' in v));
	if (keep.length === 0) return '';
	const items = keep.map(([key, value]) => `${JSON.stringify(key)}: ${JSON.stringify(value)}`);
	return `,\n    dependencies: { ${items.join(', ')} }`;
}

async function sourceManifest(name) {
	const src = join(stdDir, name, 'Project.config');
	if (!existsSync(src)) return null;
	mkdirSync(stage, { recursive: true });
	const tmp = join(stage, `${name.replace('/', '_')}.manifest.mjs`);
	writeFileSync(tmp, readFileSync(src, 'utf8'));
	try {
		const mod = await import(pathToFileURL(tmp).href);
		return mod.default ?? null;
	} finally {
		rmSync(tmp, { force: true });
	}
}

const moduleBlurb = (name) => {
	const text = readFileSync(join(stdDir, name, 'lib.rnx'), 'utf8');
	const lines = [];
	for (const line of text.split('\n')) {
		if (!line.startsWith('//!')) break;
		lines.push(line.replace(/^\/\/! ?/, ''));
	}
	while (lines.length > 0 && lines[0].trim() === '') lines.shift();
	const para = [];
	for (const line of lines) {
		if (line.trim() === '') break;
		para.push(line);
	}
	return para.join('\n');
};

rmSync(stage, { recursive: true, force: true });
let published = 0;
let skipped = 0;
// Passes repeat until everything lands: a module whose @std/* dep is not
// published yet fails with "is not preloaded" and retries on the next
// pass once its dep exists. A pass that makes no progress while modules
// remain means a real error, which throws instead of looping forever.
const isMissingDep = (text) => text.includes('is not preloaded');
let pending = [...modules];
let lastError = null;
while (pending.length > 0) {
	const next = [];
	let progressed = false;
	for (const name of pending) {
		const full = `@std/${name}`;
		const dir = join(stage, name.replace('/', '_'));
		mkdirSync(join(dir, 'src'), { recursive: true });
		cpSync(join(stdDir, name, 'lib.rnx'), join(dir, 'src', 'lib.rnx'));
		const docsDir = join(stdDir, name, 'docs');
		if (existsSync(docsDir)) cpSync(docsDir, join(dir, 'docs'), { recursive: true });
		for (const sub of SUBMODULES[name] ?? []) {
			cpSync(join(stdDir, name, sub), join(dir, 'src', sub));
		}
		if (!existsSync(join(dir, 'Project.config'))) {
			const manifest = await sourceManifest(name);
			writeFileSync(join(dir, 'README.md'), `# ${full}\n\n${moduleBlurb(name)}\n`);
			cpSync(join(repo, 'LICENSE'), join(dir, 'LICENSE'));
			writeFileSync(
				join(dir, 'Project.config'),
				`export default {\n    project: {\n        name: "${full}",\n        version: "${cliVersion}",\n        description: ${JSON.stringify(moduleBlurb(name))},\n        engine: ">=${cliVersion}"\n    },\n    entries: {\n        main: "src/lib.rnx",\n        docs: "docs"\n    }${renderDependencies(manifest)}${renderPermissions(name, manifest)}\n}\n`
			);
			rnx(['fmt', join(dir, 'Project.config')], dir);
			for (const f of [join(dir, 'src', 'lib.rnx'), ...(SUBMODULES[name] ?? []).map((s) => join(dir, 'src', s))]) {
				try {
					rnx(['fmt', '--check', f], dir);
				} catch {
					throw new Error(
						`publish-std: staged source ${f} is not fmt-clean; run \`rnx fmt\` on compiler/stdlib and recommit`
					);
				}
			}
		}
		const outDir = join(dir, 'dist');
		mkdirSync(outDir, { recursive: true });
		try {
			rnx(['publish', '--registry', SITE], dir);
			published++;
			progressed = true;
			console.log(`publish-std: published ${full}@${cliVersion}`);
		} catch (e) {
			const text = String(e.stdout ?? '') + String(e.stderr ?? '') + String(e.message ?? '');
			if (text.includes('409') || text.includes('already published')) {
				skipped++;
				progressed = true;
				console.log(`publish-std: skip ${full}@${cliVersion} (exists)`);
			} else if (isMissingDep(text)) {
				lastError = new Error(`publish-std: ${full} waiting on an unpublished dep`);
				next.push(name);
				console.log(`publish-std: defer ${full}@${cliVersion} (dep not published yet)`);
			} else {
				throw e;
			}
		}
	}
	if (!progressed) throw lastError ?? new Error('publish-std: stall with modules remaining');
	pending = next;
}
rmSync(stage, { recursive: true, force: true });
console.log(`publish-std: ${published} published, ${skipped} skipped, ${modules.length} modules`);
