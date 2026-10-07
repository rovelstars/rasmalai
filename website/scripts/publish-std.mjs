#!/usr/bin/env node
// Publishes every @std/* module as a full registry package (tarball +
// metadata), so chunks-fetch, jsdoc, and the code browser work for stdlib.
// Idempotent: already-published versions come back 409 and are skipped.
// Re-run after every release; new versions publish, old ones skip.
//
//   RNX_BIN=/path/to/rnx SITE_URL=https://... PUBLISH_TOKEN=... node scripts/publish-std.mjs
// RNX_BIN defaults to `cargo run -q -p cli` from the repo root.
import { readFileSync, readdirSync, mkdirSync, rmSync, cpSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
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
const rnxBin = process.env['RNX_BIN'];
const rnx = (args, cwd) => {
	if (rnxBin) return execFileSync(rnxBin, args, { cwd, stdio: 'pipe' });
	return execFileSync('cargo', ['run', '-q', '-p', 'cli', '--', ...args], { cwd: repo, stdio: 'pipe' });
};

const cliVersion = readFileSync(join(repo, 'compiler', 'cli', 'Cargo.toml'), 'utf8').match(
	/^version = "(.*)"$/m
)?.[1];
if (!cliVersion) throw new Error('cannot read cli version');

const modules = [];
for (const e of readdirSync(stdDir, { withFileTypes: true })) {
	if (e.isFile() && e.name.endsWith('.rnx')) modules.push(e.name.slice(0, -4));
}
modules.sort();
// net/http.rnx is a submodule file, not a package: it ships inside @std/net.
const SUBMODULES = { net: ['http.rnx'] };

// Declared permissions per std module, mirroring the server usage scan
// (website/src/lib/server/permissions.ts) over minified sources. Only
// modules whose shipped code observably uses a capability sink declare
// one; everything else publishes with no permissions key (pure).
const PERMISSIONS = {
	fs: [{ perm: 'unsafe:raw_memory', reason: 'mmap is backed by a raw address' }],
	io: [
		'term:read',
		'term:write',
		{ perm: 'env:read:NO_COLOR', reason: 'color detection reads NO_COLOR' },
		{ perm: 'env:read:COLORTERM', reason: 'color detection reads COLORTERM' },
		{ perm: 'env:read:TERM', reason: 'color detection reads TERM' }
	],
	process: [{ perm: 'env:read:*', reason: 'flattenEnv reads caller-provided keys' }]
};

function renderPermissions(name) {
	const decls = PERMISSIONS[name];
	if (!decls) return '';
	const items = decls.map((d) =>
		typeof d === 'string' ? JSON.stringify(d) : `{ perm: ${JSON.stringify(d.perm)}, reason: ${JSON.stringify(d.reason)} }`
	);
	return `,\n    permissions: [${items.join(', ')}]`;
}

const moduleBlurb = (name) => {
	const text = readFileSync(join(stdDir, `${name}.rnx`), 'utf8');
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
for (const name of modules) {
	const full = `@std/${name}`;
	const dir = join(stage, name.replace('/', '_'));
	mkdirSync(join(dir, 'src'), { recursive: true });
	cpSync(join(stdDir, `${name}.rnx`), join(dir, 'src', 'lib.rnx'));
	for (const sub of SUBMODULES[name] ?? []) {
		cpSync(join(stdDir, name, sub), join(dir, 'src', sub));
	}
	writeFileSync(join(dir, 'README.md'), `# ${full}\n\n${moduleBlurb(name)}\n`);
	writeFileSync(
		join(dir, 'Project.config'),
		`export default {\n    project: {\n        name: "${full}",\n        version: "${cliVersion}",\n        description: ${JSON.stringify(moduleBlurb(name))}\n    },\n    entries: { main: "src/lib.rnx" }${renderPermissions(name)}\n}\n`
	);
	const outDir = join(dir, 'dist');
	mkdirSync(outDir, { recursive: true });
	rnx(['pack', '--out-dir', outDir], dir);
	const tars = readdirSync(outDir).filter((f) => /\.tar(\.gz)?$|\.rnxpkg$/.test(f));
	if (tars.length === 0) throw new Error(`no archive produced for ${full}`);
	const tarball = join(outDir, tars[0]);
	try {
		rnx(['publish', tarball, '--registry', `${SITE}/api/packages`, '--token', TOKEN], dir);
		published++;
		console.log(`publish-std: published ${full}@${cliVersion}`);
	} catch (e) {
		const text = String(e.stdout ?? '') + String(e.stderr ?? '') + String(e.message ?? '');
		if (text.includes('409') || text.includes('already published')) {
			skipped++;
			console.log(`publish-std: skip ${full}@${cliVersion} (exists)`);
		} else {
			throw e;
		}
	}
}
rmSync(stage, { recursive: true, force: true });
console.log(`publish-std: ${published} published, ${skipped} skipped, ${modules.length} modules`);
